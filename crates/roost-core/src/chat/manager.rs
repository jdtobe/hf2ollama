//! The session manager: one task that owns every open session, the incognito
//! map and the rule of at most one reply per session. The GUI sends intent
//! commands and receives ordered events; all disk work goes through the store
//! writer and all HTTP through run tasks.

use super::context::{build_request, window};
use super::run::{drive, RunMsg};
use super::types::{ChatCommand, ChatEvent, EventSink, SendRejection, TurnOutcome};
use crate::error::Error;
use crate::ollama::{ChatRequestBody, OllamaClient, StreamEvent};
use crate::session::{
    AssistantStatus, MessageContent, MessageId, RunId, Session, SessionError, SessionId,
    SessionLifetime, TurnId,
};
use crate::store::{live_row, IndexRow, LoadError, StoreWarning, Writer};
use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;

pub const CHAT_SYSTEM_PROMPT: &str = "You are Roost, a helpful assistant running locally. Be concise, be honest about uncertainty, and do not claim to have used tools unless a tool result appears in this conversation.";
const FLUSH_INTERVAL: Duration = Duration::from_secs(2);
const COALESCE: Duration = Duration::from_millis(30);
const SHUTDOWN_LIMIT: Duration = Duration::from_secs(3);

pub struct ChatConfig {
    pub ollama: OllamaClient,
    /// How often a streaming reply is written to disk.
    pub flush_interval: Duration,
    /// Content deltas are batched for this long before an event is sent.
    pub coalesce: Duration,
}

impl ChatConfig {
    pub fn new(ollama: OllamaClient) -> ChatConfig {
        ChatConfig {
            ollama,
            flush_interval: FLUSH_INTERVAL,
            coalesce: COALESCE,
        }
    }
}

enum Cmd {
    User(ChatCommand),
    SetClient(Box<OllamaClient>),
    Shutdown(oneshot::Sender<()>),
}

#[derive(Clone)]
pub struct ChatHandle {
    tx: mpsc::UnboundedSender<Cmd>,
}

impl ChatHandle {
    pub fn send(&self, command: ChatCommand) {
        let _ = self.tx.send(Cmd::User(command));
    }

    /// Uses a rebuilt client (settings changed) for future requests.
    pub fn set_ollama(&self, client: OllamaClient) {
        let _ = self.tx.send(Cmd::SetClient(Box::new(client)));
    }

    /// Stops every run, writes what is partial and destroys incognito state.
    /// Gives up after three seconds.
    pub async fn shutdown(&self) {
        let (ack, done) = oneshot::channel();
        if self.tx.send(Cmd::Shutdown(ack)).is_ok() {
            let _ = tokio::time::timeout(SHUTDOWN_LIMIT, done).await;
        }
    }
}

pub struct ChatManager;

impl ChatManager {
    pub fn spawn(
        rt: &tokio::runtime::Handle,
        writer: Writer,
        cfg: ChatConfig,
        sink: EventSink,
    ) -> ChatHandle {
        let (tx, rx) = mpsc::unbounded_channel();
        let (run_tx, run_rx) = mpsc::unbounded_channel();
        let manager = Manager {
            writer,
            client: cfg.ollama,
            flush_interval: cfg.flush_interval,
            coalesce: cfg.coalesce,
            sink,
            sessions: HashMap::new(),
            ctx_cache: HashMap::new(),
            run_tx,
            next_token: 0,
        };
        rt.spawn(manager.run(rx, run_rx));
        ChatHandle { tx }
    }
}

#[derive(Clone)]
enum Intent {
    Send(String),
    Retry,
}

struct Run {
    turn: TurnId,
    assistant: MessageId,
    stop: watch::Sender<bool>,
    join: JoinHandle<()>,
    content: String,
    thinking: String,
    pending_since: Option<Instant>,
}

enum Slot {
    Idle,
    Preparing { token: u64, intent: Intent },
    Running(Box<Run>),
}

struct Live {
    s: Session,
    slot: Slot,
    close_when_idle: bool,
    last_flush: Instant,
    /// Text arrived since the last write to disk.
    dirty: bool,
}

impl Live {
    fn new(s: Session) -> Live {
        Live {
            s,
            slot: Slot::Idle,
            close_when_idle: false,
            last_flush: Instant::now(),
            dirty: false,
        }
    }
}

struct Manager {
    writer: Writer,
    client: OllamaClient,
    flush_interval: Duration,
    coalesce: Duration,
    sink: EventSink,
    sessions: HashMap<SessionId, Live>,
    /// Model name to context length from `/api/show`.
    ctx_cache: HashMap<String, Option<u64>>,
    run_tx: mpsc::UnboundedSender<RunMsg>,
    next_token: u64,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

fn error_text(e: &Error) -> String {
    match e {
        Error::OllamaUnreachable => {
            "Ollama is not reachable. Make sure it is running, then retry.".into()
        }
        Error::ModelNotFound(m) => format!("The model {m} is not installed. Choose another model."),
        Error::Network(m) if m == "ollama stalled" => {
            "Ollama stopped responding. Retry when it is ready.".into()
        }
        other => other.to_string(),
    }
}

fn warning_text(w: &StoreWarning) -> String {
    match w {
        StoreWarning::IndexRebuilt => {
            "The history index was rebuilt from the session files.".into()
        }
        StoreWarning::Quarantined(_) => {
            "A damaged session file was set aside and kept for diagnosis.".into()
        }
        StoreWarning::NewerSchema(_) => {
            "A session was made by a newer Roost and was left untouched.".into()
        }
        StoreWarning::Unreadable(_) => "A session file could not be read right now.".into(),
    }
}

fn load_error_text(e: &LoadError) -> String {
    e.to_string()
}

async fn sleep_until_opt(deadline: Option<Instant>) {
    match deadline {
        Some(d) => tokio::time::sleep_until(d).await,
        None => std::future::pending().await,
    }
}

impl Manager {
    fn emit(&self, ev: ChatEvent) {
        (self.sink)(ev);
    }

    async fn emit_rows(&self) {
        let mut rows: Vec<IndexRow> = self.writer.snapshot().await;
        rows.extend(
            self.sessions
                .values()
                .filter(|l| l.s.lifetime == SessionLifetime::Incognito)
                .map(|l| live_row(&l.s)),
        );
        self.emit(ChatEvent::Rows(rows));
    }

    fn save(&self, s: &Session) {
        if s.lifetime == SessionLifetime::Persisted {
            self.writer.save(s.clone());
        }
    }

    async fn run(
        mut self,
        mut cmd_rx: mpsc::UnboundedReceiver<Cmd>,
        mut run_rx: mpsc::UnboundedReceiver<RunMsg>,
    ) {
        for w in self.writer.take_warnings() {
            self.emit(ChatEvent::StoreWarning(warning_text(&w)));
        }
        self.emit_rows().await;
        loop {
            let deadline = self.next_deadline();
            tokio::select! {
                cmd = cmd_rx.recv() => match cmd {
                    Some(c) => if self.on_cmd(c).await { break },
                    None => break,
                },
                Some(msg) = run_rx.recv() => self.on_run_msg(msg).await,
                _ = sleep_until_opt(deadline) => self.on_tick(),
            }
            for e in self.writer.take_errors() {
                self.emit(ChatEvent::SessionError {
                    session: None,
                    message: e,
                });
            }
        }
    }

    /// Returns true when the manager should stop.
    async fn on_cmd(&mut self, cmd: Cmd) -> bool {
        match cmd {
            Cmd::SetClient(c) => self.client = *c,
            Cmd::Shutdown(ack) => {
                self.shutdown().await;
                let _ = ack.send(());
                return true;
            }
            Cmd::User(c) => self.on_command(c).await,
        }
        false
    }

    async fn on_command(&mut self, cmd: ChatCommand) {
        match cmd {
            ChatCommand::Create { lifetime, model } => self.create(lifetime, &model).await,
            ChatCommand::Open(id) => self.open(id).await,
            ChatCommand::Close(id) => self.close(id).await,
            ChatCommand::ClearIncognito => self.clear_incognito().await,
            ChatCommand::Rename(id, title) => {
                self.edit(id, move |s| s.rename(&title)).await;
            }
            ChatCommand::SetPinned(id, pinned) => {
                self.edit(id, move |s| {
                    s.set_pinned(pinned);
                    Ok(())
                })
                .await;
            }
            ChatCommand::SetModel(id, model) => self.set_model(id, model).await,
            ChatCommand::Delete(id) => self.delete(id).await,
            ChatCommand::Send { session, text } => self.begin(session, Intent::Send(text)).await,
            ChatCommand::Retry(id) => self.begin(id, Intent::Retry).await,
            ChatCommand::Stop(id) => self.stop(id).await,
            ChatCommand::ListSessions => self.emit_rows().await,
            ChatCommand::InstalledChanged => self.ctx_cache.clear(),
        }
    }

    fn session_error(&self, id: Option<SessionId>, message: String) {
        self.emit(ChatEvent::SessionError {
            session: id,
            message,
        });
    }

    /// Loads a saved session into memory if it is not there already.
    async fn ensure(&mut self, id: SessionId) -> Result<(), LoadError> {
        if self.sessions.contains_key(&id) {
            return Ok(());
        }
        let s = self.writer.load(id).await?;
        self.sessions.insert(id, Live::new(s));
        Ok(())
    }

    async fn create(&mut self, lifetime: SessionLifetime, model: &str) {
        let s = Session::new_chat(model, lifetime, now_ms());
        self.save(&s);
        self.emit(ChatEvent::Opened(Box::new(s.clone())));
        self.sessions.insert(s.id, Live::new(s));
        self.emit_rows().await;
    }

    async fn open(&mut self, id: SessionId) {
        if let Err(e) = self.ensure(id).await {
            self.session_error(Some(id), load_error_text(&e));
            self.emit_rows().await;
            return;
        }
        let mut pending = None;
        let mut snapshot = None;
        if let Some(l) = self.sessions.get_mut(&id) {
            l.close_when_idle = false;
            // Text still waiting to be batched is already in the snapshot, so
            // send it first; otherwise the viewer would append it twice.
            if let Slot::Running(r) = &mut l.slot {
                if !r.content.is_empty() || !r.thinking.is_empty() {
                    pending = Some((
                        r.turn,
                        std::mem::take(&mut r.content),
                        std::mem::take(&mut r.thinking),
                    ));
                }
                r.pending_since = None;
            }
            snapshot = Some(l.s.clone());
        }
        if let Some((turn, content, thinking)) = pending {
            self.emit(ChatEvent::Delta {
                session: id,
                turn,
                content,
                thinking,
            });
        }
        if let Some(s) = snapshot {
            self.emit(ChatEvent::Opened(Box::new(s)));
        }
    }

    async fn close(&mut self, id: SessionId) {
        let Some(live) = self.sessions.get_mut(&id) else {
            return;
        };
        if live.s.lifetime == SessionLifetime::Incognito {
            if let Some(live) = self.sessions.remove(&id) {
                self.halt(live).await;
            }
            self.emit_rows().await;
        } else if matches!(live.slot, Slot::Idle) {
            self.sessions.remove(&id);
        } else {
            live.close_when_idle = true;
        }
    }

    async fn clear_incognito(&mut self) {
        let ids: Vec<SessionId> = self
            .sessions
            .iter()
            .filter(|(_, l)| l.s.lifetime == SessionLifetime::Incognito)
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            if let Some(live) = self.sessions.remove(&id) {
                self.halt(live).await;
            }
        }
        self.emit_rows().await;
    }

    /// Edits a session (loading it if needed), saves it and refreshes rows.
    async fn edit<F>(&mut self, id: SessionId, f: F)
    where
        F: FnOnce(&mut Session) -> Result<(), SessionError>,
    {
        let was_loaded = self.sessions.contains_key(&id);
        if let Err(e) = self.ensure(id).await {
            self.session_error(Some(id), load_error_text(&e));
            return;
        }
        let mut failed = None;
        if let Some(live) = self.sessions.get_mut(&id) {
            match f(&mut live.s) {
                Ok(()) => {
                    if live.s.lifetime == SessionLifetime::Persisted {
                        self.writer.save(live.s.clone());
                    }
                }
                Err(e) => failed = Some(e.to_string()),
            }
            if !was_loaded && matches!(live.slot, Slot::Idle) {
                self.sessions.remove(&id);
            }
        }
        if let Some(m) = failed {
            self.session_error(Some(id), m);
        }
        self.emit_rows().await;
    }

    async fn set_model(&mut self, id: SessionId, model: String) {
        if let Some(live) = self.sessions.get(&id) {
            if !matches!(live.slot, Slot::Idle) {
                self.session_error(
                    Some(id),
                    "Stop the current reply before changing the model.".into(),
                );
                return;
            }
        }
        self.edit(id, move |s| {
            s.model = model;
            Ok(())
        })
        .await;
    }

    async fn delete(&mut self, id: SessionId) {
        let mut incognito = false;
        if let Some(live) = self.sessions.remove(&id) {
            incognito = live.s.lifetime == SessionLifetime::Incognito;
            self.halt(live).await;
        }
        if !incognito {
            self.writer.delete(id);
        }
        self.emit_rows().await;
    }

    /// Tells a run to stop and waits briefly for its task to finish.
    async fn halt(&self, live: Live) {
        // The run task owns no session state and the manager ignores
        // anything it sends later, so it can simply be cut off.
        if let Slot::Running(run) = live.slot {
            let _ = run.stop.send(true);
            run.join.abort();
        }
    }

    async fn begin(&mut self, id: SessionId, intent: Intent) {
        if !self.sessions.contains_key(&id) && self.ensure(id).await.is_err() {
            self.emit(ChatEvent::SendRejected {
                session: id,
                reason: SendRejection::UnknownSession,
            });
            return;
        }
        let Some(live) = self.sessions.get(&id) else {
            return;
        };
        if !matches!(live.slot, Slot::Idle) {
            self.emit(ChatEvent::SendRejected {
                session: id,
                reason: SendRejection::Busy,
            });
            return;
        }
        match &intent {
            Intent::Send(text) if text.trim().is_empty() => {
                self.emit(ChatEvent::SendRejected {
                    session: id,
                    reason: SendRejection::EmptyMessage,
                });
                return;
            }
            Intent::Retry => {
                let retryable = matches!(
                    live.s.messages.last().map(|m| &m.content),
                    Some(MessageContent::Assistant {
                        status: AssistantStatus::Failed
                            | AssistantStatus::Cancelled
                            | AssistantStatus::Interrupted,
                        ..
                    })
                );
                if !retryable {
                    self.session_error(Some(id), "There is nothing to retry.".into());
                    return;
                }
            }
            Intent::Send(_) => {}
        }
        let model = live.s.model.clone();
        if let Some(ctx) = self.ctx_cache.get(&model).copied() {
            self.proceed(id, intent, ctx).await;
            return;
        }
        self.next_token += 1;
        let token = self.next_token;
        if let Some(live) = self.sessions.get_mut(&id) {
            live.slot = Slot::Preparing { token, intent };
        }
        let (client, tx) = (self.client.clone(), self.run_tx.clone());
        tokio::spawn(async move {
            let context = client.show(&model).await.map(|d| d.context);
            let _ = tx.send(RunMsg::Shown {
                session: id,
                token,
                context,
            });
        });
    }

    /// The model's context length is known (or unknown): trim, record the
    /// user message, and start the reply.
    async fn proceed(&mut self, id: SessionId, intent: Intent, show_ctx: Option<u64>) {
        let Some(live) = self.sessions.get_mut(&id) else {
            return;
        };
        let now = now_ms();
        let before = live.s.updated_at_ms;
        let user = match &intent {
            Intent::Send(text) => {
                let uid = live.s.push_user(text, now);
                live.s.messages.iter().find(|m| m.id == uid).cloned()
            }
            Intent::Retry => None,
        };
        let win = window(live.s.model_options.num_ctx, show_ctx);
        let trimmed = match build_request(CHAT_SYSTEM_PROMPT, &live.s.messages, win) {
            Ok(t) => t,
            Err(e) => {
                if user.is_some() {
                    live.s.messages.pop();
                    live.s.updated_at_ms = before;
                }
                live.slot = Slot::Idle;
                let close = live.close_when_idle;
                self.emit(ChatEvent::SendRejected {
                    session: id,
                    reason: SendRejection::TooLarge(e.to_string()),
                });
                if close {
                    self.sessions.remove(&id);
                }
                return;
            }
        };
        let (run_id, turn) = (RunId::new(), TurnId::new());
        let assistant = live.s.begin_assistant(run_id, turn, now);
        let body = ChatRequestBody {
            model: live.s.model.clone(),
            messages: trimmed.messages,
            num_ctx: win,
            temperature: live.s.model_options.temperature,
            seed: live.s.model_options.seed,
            think: live.s.model_options.think,
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
        if let Some(m) = user {
            self.emit(ChatEvent::Appended {
                session: id,
                message: Box::new(m),
            });
        }
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
        let others_running = self
            .sessions
            .iter()
            .any(|(k, l)| *k != id && matches!(l.slot, Slot::Running(_)));
        if others_running {
            self.emit(ChatEvent::WaitingForModel { session: id, turn });
        }
        self.emit_rows().await;
    }

    async fn stop(&mut self, id: SessionId) {
        let Some(live) = self.sessions.get_mut(&id) else {
            return;
        };
        match live.slot {
            Slot::Running(_) => self.finalize(id, TurnOutcome::Cancelled, None, None).await,
            Slot::Preparing { .. } => {
                live.slot = Slot::Idle;
                self.emit(ChatEvent::SendRejected {
                    session: id,
                    reason: SendRejection::Cancelled,
                });
            }
            Slot::Idle => {}
        }
    }

    async fn on_run_msg(&mut self, msg: RunMsg) {
        match msg {
            RunMsg::Shown {
                session,
                token,
                context,
            } => {
                let Some(live) = self.sessions.get_mut(&session) else {
                    return;
                };
                let Slot::Preparing {
                    token: t,
                    ref intent,
                } = live.slot
                else {
                    return;
                };
                if t != token {
                    return;
                }
                let intent = intent.clone();
                live.slot = Slot::Idle;
                let model = live.s.model.clone();
                let ctx = match context {
                    Ok(c) => {
                        self.ctx_cache.insert(model, c);
                        c
                    }
                    Err(_) => None,
                };
                self.proceed(session, intent, ctx).await;
            }
            RunMsg::Event {
                session,
                turn,
                event,
            } => {
                let Some(live) = self.sessions.get_mut(&session) else {
                    return;
                };
                let Slot::Running(run) = &mut live.slot else {
                    return;
                };
                if run.turn != turn {
                    return;
                }
                match event {
                    StreamEvent::Content(c) => {
                        live.dirty = true;
                        live.s.append_assistant(run.assistant, &c, "");
                        run.content.push_str(&c);
                    }
                    StreamEvent::Thinking(t) => {
                        live.dirty = true;
                        live.s.append_assistant(run.assistant, "", &t);
                        run.thinking.push_str(&t);
                    }
                    StreamEvent::Done {
                        prompt_eval_count,
                        done_reason,
                        ..
                    } => {
                        // A reply that hit the window or output limit is cut
                        // off, not finished: keep the text, offer Retry.
                        let (outcome, message) = if done_reason.as_deref() == Some("length") {
                            (
                                TurnOutcome::Failed,
                                Some(
                                    "The reply was cut off because it reached the length limit."
                                        .to_string(),
                                ),
                            )
                        } else {
                            (TurnOutcome::Completed, None)
                        };
                        self.finalize(session, outcome, message, prompt_eval_count)
                            .await;
                        return;
                    }
                }
                if run.pending_since.is_none() {
                    run.pending_since = Some(Instant::now());
                }
            }
            RunMsg::Failed {
                session,
                turn,
                error,
            } => {
                let current = matches!(
                    self.sessions.get(&session).map(|l| &l.slot),
                    Some(Slot::Running(r)) if r.turn == turn
                );
                if !current {
                    return;
                }
                if let Error::ModelNotFound(m) = &error {
                    self.emit(ChatEvent::ModelMissing {
                        session,
                        model: m.clone(),
                    });
                }
                self.finalize(session, TurnOutcome::Failed, Some(error_text(&error)), None)
                    .await;
            }
        }
    }

    fn flush_pending(&self, id: SessionId, run: &mut Run) {
        if run.content.is_empty() && run.thinking.is_empty() {
            run.pending_since = None;
            return;
        }
        self.emit(ChatEvent::Delta {
            session: id,
            turn: run.turn,
            content: std::mem::take(&mut run.content),
            thinking: std::mem::take(&mut run.thinking),
        });
        run.pending_since = None;
    }

    /// Records the turn's single terminal outcome. Anything the run task
    /// sends afterwards is ignored because the slot is no longer running.
    async fn finalize(
        &mut self,
        id: SessionId,
        outcome: TurnOutcome,
        message: Option<String>,
        prompt_eval_count: Option<u64>,
    ) {
        let Some(live) = self.sessions.get_mut(&id) else {
            return;
        };
        let Slot::Running(mut run) = std::mem::replace(&mut live.slot, Slot::Idle) else {
            return;
        };
        let _ = run.stop.send(true);
        let (status, error) = match outcome {
            TurnOutcome::Completed => (AssistantStatus::Complete, None),
            TurnOutcome::Failed => (AssistantStatus::Failed, message.clone()),
            TurnOutcome::Cancelled => (AssistantStatus::Cancelled, None),
        };
        live.s
            .finish_assistant(run.assistant, status, error, now_ms());
        let close = live.close_when_idle;
        let s = live.s.clone();
        self.flush_pending(id, &mut run);
        self.save(&s);
        self.emit(ChatEvent::TurnEnded {
            session: id,
            turn: run.turn,
            outcome,
            message,
            prompt_eval_count,
        });
        if close {
            self.sessions.remove(&id);
        }
        self.emit_rows().await;
    }

    fn next_deadline(&self) -> Option<Instant> {
        self.sessions
            .values()
            .filter_map(|l| match &l.slot {
                Slot::Running(r) => {
                    let flush = l.last_flush + self.flush_interval;
                    Some(match r.pending_since {
                        Some(p) => flush.min(p + self.coalesce),
                        None => flush,
                    })
                }
                _ => None,
            })
            .min()
    }

    fn on_tick(&mut self) {
        let now = Instant::now();
        let (coalesce, flush_every) = (self.coalesce, self.flush_interval);
        let ids: Vec<SessionId> = self.sessions.keys().copied().collect();
        for id in ids {
            let mut deltas = None;
            let mut save = None;
            if let Some(l) = self.sessions.get_mut(&id) {
                if let Slot::Running(r) = &mut l.slot {
                    if r.pending_since.is_some_and(|p| now >= p + coalesce) {
                        deltas = Some((
                            r.turn,
                            std::mem::take(&mut r.content),
                            std::mem::take(&mut r.thinking),
                        ));
                        r.pending_since = None;
                    }
                    if now >= l.last_flush + flush_every {
                        l.last_flush = now;
                        if std::mem::take(&mut l.dirty) {
                            save = Some(l.s.clone());
                        }
                    }
                }
            }
            if let Some((turn, content, thinking)) = deltas {
                self.emit(ChatEvent::Delta {
                    session: id,
                    turn,
                    content,
                    thinking,
                });
            }
            if let Some(s) = save {
                self.save(&s);
            }
        }
    }

    async fn shutdown(&mut self) {
        let running: Vec<SessionId> = self
            .sessions
            .iter()
            .filter(|(_, l)| matches!(l.slot, Slot::Running(_)))
            .map(|(id, _)| *id)
            .collect();
        for id in running {
            self.finalize(id, TurnOutcome::Cancelled, None, None).await;
        }
        self.sessions
            .retain(|_, l| l.s.lifetime != SessionLifetime::Incognito);
        self.writer.flush().await;
    }
}
