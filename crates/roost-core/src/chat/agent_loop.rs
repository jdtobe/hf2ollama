//! Agent runs: the model/tool step loop, steering and the step budget.
//!
//! A run is a series of steps. Each step streams one model reply; if the reply
//! asks for tools they run one at a time and the loop goes again, until the
//! model answers without tools, the user stops it, or the steps run out.
//! Steering messages wait in an inbox and are applied at the next safe point.

use super::context::{build_agent_request, window};
use super::manager::{now_ms, Intent, Manager, ModelInfo, Run, Slot, CHAT_SYSTEM_PROMPT};
use super::run::{drive, RunMsg};
use super::types::{ChatEvent, RunOutcome, SendRejection, TurnOutcome};
use crate::agent::registry;
use crate::agent::tools::Limits;
use crate::agent::{ToolError, Workspace};
use crate::ollama::ChatRequestBody;
use crate::session::{
    MessageId, RunId, Session, SessionId, SessionLifetime, SessionMode, ToolStatus, TurnId,
};
use serde_json::Value;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::Instant;

pub const AGENT_PROMPT_ADDITION: &str = "You are operating as a read-only agent inside the attached workspace. Use only the provided tools, stay within the workspace, never invent tool results, honor steering messages, and stop when the task is complete or the step budget is exhausted.";

struct PendingCall {
    call_id: String,
    name: String,
    arguments: Value,
    msg: MessageId,
}

/// The loop state of one active agent run.
pub(super) struct AgentRun {
    run: RunId,
    steps: u32,
    /// Tool calls the current reply has asked for so far.
    collected: Vec<(String, Value)>,
    queue: VecDeque<PendingCall>,
    inbox: VecDeque<String>,
    workspace: Arc<Workspace>,
    next_call: u32,
}

impl AgentRun {
    pub(super) fn collect(&mut self, name: String, arguments: Value) {
        self.collected.push((name, arguments));
    }
}

/// A tool call running on the blocking pool.
pub(super) struct ToolRun {
    pub(super) join: JoinHandle<()>,
    call_id: String,
    msg: MessageId,
}

/// The system prompt for one agent step, with the workspace, the tools that
/// are on and the steps left appended as structured context.
fn system_prompt(s: &Session, steps_left: u32) -> String {
    let tools: Vec<&str> = registry::offered(&s.effective_tools)
        .iter()
        .map(|t| t.name)
        .collect();
    let workspace = s
        .workspace
        .as_ref()
        .map_or_else(String::new, |p| p.display().to_string());
    format!(
        "{CHAT_SYSTEM_PROMPT} {AGENT_PROMPT_ADDITION}\n\nWorkspace: {workspace}\nTools: {}\nSteps remaining: {steps_left}",
        if tools.is_empty() {
            "none".to_string()
        } else {
            tools.join(", ")
        }
    )
}

impl Manager {
    // ------------------------------------------------------------ creating

    pub(super) async fn create_agent(
        &mut self,
        lifetime: SessionLifetime,
        model: &str,
        workspace: PathBuf,
    ) {
        if let Err(e) = Workspace::open(&workspace) {
            self.session_error(None, format!("That folder cannot be used: {e}"));
            return;
        }
        let s = match Session::new_agent(model, lifetime, workspace, now_ms()) {
            Ok(s) => s,
            Err(e) => {
                self.session_error(None, e.to_string());
                return;
            }
        };
        self.save(&s);
        self.emit(ChatEvent::Opened(Box::new(s.clone())));
        self.sessions.insert(s.id, super::manager::Live::new(s));
        self.emit_rows().await;
    }

    /// Answers `CheckModel` from the cache, or asks Ollama.
    pub(super) fn check_model(&mut self, model: String) {
        if let Some(info) = self.ctx_cache.get(&model) {
            self.emit(ChatEvent::ModelCapabilities {
                tools: info.tools,
                model,
            });
            return;
        }
        let (client, tx) = (self.client.clone(), self.run_tx.clone());
        tokio::spawn(async move {
            let info = client.show(&model).await.map(|d| ModelInfo {
                context: d.context,
                tools: d.capabilities.iter().any(|c| c == "tools"),
            });
            let _ = tx.send(RunMsg::Capabilities { model, info });
        });
    }

    pub(super) async fn set_workspace(&mut self, id: SessionId, path: PathBuf) {
        if let Err(e) = Workspace::open(&path) {
            self.session_error(Some(id), format!("That folder cannot be used: {e}"));
            return;
        }
        self.edit_idle(id, move |s| s.set_workspace(path)).await;
    }

    /// Edits settings that must not change under a running agent.
    pub(super) async fn edit_idle<F>(&mut self, id: SessionId, f: F)
    where
        F: FnOnce(&mut Session) -> Result<(), crate::session::SessionError>,
    {
        if self
            .sessions
            .get(&id)
            .is_some_and(|l| !matches!(l.slot, Slot::Idle))
        {
            self.session_error(
                Some(id),
                "Stop the current run before changing this.".into(),
            );
            return;
        }
        self.edit(id, f).await;
        if let Some(live) = self
            .sessions
            .get(&id)
            .filter(|l| l.s.mode == SessionMode::Agent)
        {
            self.emit(ChatEvent::AgentSettings {
                session: id,
                workspace: live.s.workspace.clone(),
                step_budget: live.s.step_budget,
                tools: live.s.effective_tools.clone(),
            });
        }
    }

    // ------------------------------------------------------------ starting

    /// The model's capabilities are known: check the preconditions, record
    /// the user's message and start the loop.
    pub(super) async fn agent_proceed(
        &mut self,
        id: SessionId,
        intent: Intent,
        info: Option<ModelInfo>,
    ) {
        let Some(live) = self.sessions.get_mut(&id) else {
            return;
        };
        let reject = |this: &Self, reason| {
            this.emit(ChatEvent::SendRejected {
                session: id,
                reason,
            })
        };
        if let Some(i) = info {
            if !i.tools {
                let model = live.s.model.clone();
                live.slot = Slot::Idle;
                reject(self, SendRejection::NoToolSupport(model));
                return;
            }
        }
        let opened = match live.s.workspace.as_deref().map(Workspace::open) {
            Some(Ok(w)) => Arc::new(w),
            Some(Err(e)) => {
                live.slot = Slot::Idle;
                reject(self, SendRejection::NoWorkspace(e.to_string()));
                return;
            }
            None => {
                live.slot = Slot::Idle;
                reject(
                    self,
                    SendRejection::NoWorkspace("this agent has no workspace folder".into()),
                );
                return;
            }
        };
        let user = match &intent {
            Intent::Send(text) => {
                let uid = live.s.push_user(text, now_ms());
                live.s.messages.iter().find(|m| m.id == uid).cloned()
            }
            Intent::Retry | Intent::Regenerate => None,
        };
        live.agent = Some(AgentRun {
            run: RunId::new(),
            steps: 0,
            collected: Vec::new(),
            queue: VecDeque::new(),
            inbox: VecDeque::new(),
            workspace: opened,
            next_call: 0,
        });
        let s = live.s.clone();
        self.save(&s);
        if let Some(m) = user {
            self.emit(ChatEvent::Appended {
                session: id,
                message: Box::new(m),
            });
        }
        self.agent_step(id).await;
    }

    /// Streams one model reply for the run.
    async fn agent_step(&mut self, id: SessionId) {
        let Some(live) = self.sessions.get_mut(&id) else {
            return;
        };
        let Some(agent) = live.agent.as_mut() else {
            return;
        };
        agent.steps += 1;
        let (run_id, steps_before) = (agent.run, agent.steps - 1);
        let left = live.s.step_budget.saturating_sub(steps_before);
        let system = system_prompt(&live.s, left);
        let context = self.ctx_cache.get(&live.s.model).and_then(|i| i.context);
        let win = window(live.s.model_options.num_ctx, context);
        let trimmed = match build_agent_request(&system, &live.s.messages, win) {
            Ok(t) => t,
            Err(e) => {
                let message = e.to_string();
                self.end_run(id, RunOutcome::Failed, Some(message)).await;
                return;
            }
        };
        let turn = TurnId::new();
        let assistant = live.s.begin_assistant(run_id, turn, now_ms());
        let tools = registry::ollama_definitions(&live.s.effective_tools);
        let body = ChatRequestBody {
            model: live.s.model.clone(),
            messages: trimmed.messages,
            num_ctx: win,
            temperature: live.s.model_options.temperature,
            seed: live.s.model_options.seed,
            think: live.s.model_options.think,
            tools: tools
                .as_array()
                .is_some_and(|a| !a.is_empty())
                .then_some(tools),
        };
        let (stop_tx, stop_rx) = watch::channel(false);
        let join = tokio::spawn(drive(
            self.client.clone(),
            body,
            id,
            turn,
            self.run_tx.clone(),
            stop_rx,
        ));
        live.slot = Slot::Running(Box::new(Run {
            turn,
            assistant,
            stop: stop_tx,
            join,
            content: String::new(),
            thinking: String::new(),
            pending_since: None,
        }));
        live.last_flush = Instant::now();
        let session = live.s.clone();
        let assistant_msg = session.messages.iter().find(|m| m.id == assistant).cloned();
        self.save(&session);
        if let Some(m) = assistant_msg {
            self.emit(ChatEvent::Appended {
                session: id,
                message: Box::new(m),
            });
        }
        self.emit(ChatEvent::TurnStarted { session: id, turn });
        if trimmed.omitted > 0 {
            self.emit(ChatEvent::ContextTrimmed {
                session: id,
                omitted: trimmed.omitted,
            });
        }
        self.emit_rows().await;
    }

    // ------------------------------------------------------------- stepping

    /// A reply finished streaming. Run its tools, or finish the run.
    pub(super) async fn agent_step_done(
        &mut self,
        id: SessionId,
        outcome: TurnOutcome,
        message: Option<String>,
        prompt_eval_count: Option<u64>,
    ) {
        let calls = self
            .sessions
            .get_mut(&id)
            .and_then(|l| l.agent.as_mut())
            .map(|a| std::mem::take(&mut a.collected))
            .unwrap_or_default();
        if outcome == TurnOutcome::Failed {
            // Cut off by the length limit: keep the text, end the run.
            self.finalize(id, outcome, message, prompt_eval_count).await;
            return;
        }
        if !self
            .finalize_turn(id, outcome, message, prompt_eval_count)
            .await
        {
            return;
        }
        if calls.is_empty() {
            let steering = self
                .sessions
                .get(&id)
                .and_then(|l| l.agent.as_ref())
                .is_some_and(|a| !a.inbox.is_empty());
            if steering {
                self.agent_next(id).await;
            } else {
                self.end_run(id, RunOutcome::Completed, None).await;
            }
            return;
        }
        // Record every call first so the transcript keeps them together.
        let mut recorded = Vec::new();
        if let Some(live) = self.sessions.get_mut(&id) {
            if let Some(agent) = live.agent.as_mut() {
                for (name, arguments) in calls {
                    agent.next_call += 1;
                    let call_id = format!("call_{}", agent.next_call);
                    let msg = live.s.push_tool_call(
                        agent.run,
                        &call_id,
                        &name,
                        arguments.clone(),
                        now_ms(),
                    );
                    if let Some(m) = live.s.messages.iter().find(|m| m.id == msg).cloned() {
                        recorded.push(m);
                    }
                    agent.queue.push_back(PendingCall {
                        call_id,
                        name,
                        arguments,
                        msg,
                    });
                }
            }
            let s = live.s.clone();
            self.save(&s);
        }
        for m in recorded {
            self.emit(ChatEvent::Appended {
                session: id,
                message: Box::new(m),
            });
        }
        self.run_next_tool(id).await;
    }

    /// Starts the next queued tool call, or moves on when none are left.
    async fn run_next_tool(&mut self, id: SessionId) {
        let Some(live) = self.sessions.get_mut(&id) else {
            return;
        };
        let Some(agent) = live.agent.as_mut() else {
            return;
        };
        let Some(call) = agent.queue.pop_front() else {
            self.agent_next(id).await;
            return;
        };
        live.s.start_tool(call.msg);
        let (workspace, config, run) = (
            Arc::clone(&agent.workspace),
            live.s.effective_tools.clone(),
            agent.run,
        );
        let (tx, name, args, call_id) = (
            self.run_tx.clone(),
            call.name.clone(),
            call.arguments.clone(),
            call.call_id.clone(),
        );
        let join = tokio::task::spawn_blocking(move || {
            let result = registry::call(&workspace, &config, &name, &args, &Limits::default());
            let _ = tx.send(RunMsg::ToolDone {
                session: id,
                run,
                call_id,
                result,
            });
        });
        live.slot = Slot::Tools(Box::new(ToolRun {
            join,
            call_id: call.call_id.clone(),
            msg: call.msg,
        }));
        self.emit(ChatEvent::ToolStarted {
            session: id,
            call_id: call.call_id,
            name: call.name,
            arguments: call.arguments,
        });
    }

    pub(super) async fn tool_done(
        &mut self,
        id: SessionId,
        run: RunId,
        call_id: String,
        result: Result<String, ToolError>,
    ) {
        let Some(live) = self.sessions.get_mut(&id) else {
            return;
        };
        let msg = match &live.slot {
            Slot::Tools(t) if t.call_id == call_id => t.msg,
            _ => return,
        };
        if live.agent.as_ref().map(|a| a.run) != Some(run) {
            return;
        }
        live.slot = Slot::Idle;
        let (ok, output) = match result {
            Ok(text) => (true, text),
            Err(e) => (false, format!("Error: {e}")),
        };
        let status = if ok {
            ToolStatus::Complete
        } else {
            ToolStatus::Failed
        };
        live.s.finish_tool(msg, &output, status, now_ms());
        let s = live.s.clone();
        self.save(&s);
        self.emit(ChatEvent::ToolFinished {
            session: id,
            call_id,
            ok,
            output,
        });
        self.run_next_tool(id).await;
    }

    /// A safe point (no reply streaming, no tool running): apply waiting
    /// steering messages, then go again unless the steps are used up.
    async fn agent_next(&mut self, id: SessionId) {
        let mut appended = Vec::new();
        let (steps, budget) = {
            let Some(live) = self.sessions.get_mut(&id) else {
                return;
            };
            let Some(agent) = live.agent.as_mut() else {
                return;
            };
            while let Some(text) = agent.inbox.pop_front() {
                let mid = live.s.push_steering(&text, now_ms());
                if let Some(m) = live.s.messages.iter().find(|m| m.id == mid).cloned() {
                    appended.push(m);
                }
            }
            (agent.steps, live.s.step_budget)
        };
        if !appended.is_empty() {
            if let Some(live) = self.sessions.get(&id) {
                let s = live.s.clone();
                self.save(&s);
            }
        }
        for m in appended {
            self.emit(ChatEvent::Appended {
                session: id,
                message: Box::new(m),
            });
        }
        if steps >= budget {
            let message = format!("Stopped after using all {budget} steps.");
            self.end_run(id, RunOutcome::BudgetExhausted, Some(message))
                .await;
            return;
        }
        self.agent_step(id).await;
    }

    // ------------------------------------------------------------- steering

    /// The user sent a message to a running agent. It waits for the next
    /// safe point; a reply that is streaming is interrupted (and kept).
    pub(super) async fn steer(&mut self, id: SessionId, text: String) {
        let streaming = {
            let Some(live) = self.sessions.get_mut(&id) else {
                return;
            };
            let Some(agent) = live.agent.as_mut() else {
                return;
            };
            agent.inbox.push_back(text.clone());
            matches!(live.slot, Slot::Running(_))
        };
        self.emit(ChatEvent::SteeringQueued { session: id, text });
        if !streaming {
            return; // a tool is running; its result is the next safe point
        }
        if let Some(agent) = self.sessions.get_mut(&id).and_then(|l| l.agent.as_mut()) {
            agent.collected.clear();
        }
        if self
            .finalize_turn(id, TurnOutcome::Interrupted, None, None)
            .await
        {
            self.agent_next(id).await;
        }
    }

    // ------------------------------------------------------------- stopping

    /// Stop: cancel whatever is running and end the run.
    pub(super) async fn stop_run(&mut self, id: SessionId) {
        let Some(live) = self.sessions.get_mut(&id) else {
            return;
        };
        match &live.slot {
            Slot::Running(_) => {
                if !self
                    .finalize_turn(id, TurnOutcome::Cancelled, None, None)
                    .await
                {
                    return;
                }
            }
            Slot::Tools(_) => {
                if let Slot::Tools(t) = std::mem::replace(&mut live.slot, Slot::Idle) {
                    t.join.abort();
                }
            }
            _ => return,
        }
        if let Some(live) = self.sessions.get_mut(&id) {
            live.s.cancel_open_tools(now_ms());
            let s = live.s.clone();
            self.save(&s);
        }
        self.end_run(id, RunOutcome::Cancelled, None).await;
    }
}
