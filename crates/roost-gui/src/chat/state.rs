//! Chat view-model: folds `ChatEvent`s into plain data. No egui types.

use roost_core::chat::{ChatCommand, ChatEvent, RunOutcome, SendRejection, TurnOutcome};
use roost_core::session::{
    AssistantStatus, Message, MessageContent, MessageId, Session, SessionId, SessionLifetime,
    SessionMode, ToolStatus, TurnId, UserDelivery,
};
use roost_core::store::{IndexRow, RowStatus};
use std::collections::{HashMap, HashSet};

const DAY_MS: u64 = 86_400_000;
const MAX_WARNINGS: usize = 5;

/// The local calendar day of a timestamp, as a day number.
pub fn local_day(ms: u64) -> i64 {
    use chrono::{Datelike, TimeZone};
    chrono::Local
        .timestamp_millis_opt(ms as i64)
        .single()
        .map_or((ms / DAY_MS) as i64, |d| {
            i64::from(d.date_naive().num_days_from_ce())
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunBadge {
    None,
    Running,
    Waiting,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RailRow {
    pub id: SessionId,
    pub title: String,
    pub model: String,
    pub pinned: bool,
    pub badge: RunBadge,
    pub status: RowStatus,
    pub lifetime: SessionLifetime,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rail {
    pub pinned: Vec<RailRow>,
    pub today: Vec<RailRow>,
    pub yesterday: Vec<RailRow>,
    pub earlier: Vec<RailRow>,
    pub incognito: Vec<RailRow>,
}

/// What pressing Send did.
#[derive(Debug, Clone)]
pub enum SendAttempt {
    Sent(ChatCommand),
    /// A reply is already running or a send is in flight; the draft is kept.
    Busy,
    Empty,
}

/// What the rail shows for a chat: its title, or until it has one the start
/// of its first message.
pub fn display_title(row: &IndexRow) -> String {
    match row.preview.as_deref() {
        Some(p) if row.title == roost_core::session::UNTITLED && !p.is_empty() => p.to_string(),
        _ => row.title.clone(),
    }
}

/// The label a partial or failed reply carries, if it needs one.
pub fn status_label(status: AssistantStatus) -> Option<&'static str> {
    match status {
        AssistantStatus::Failed => Some("Failed"),
        AssistantStatus::Cancelled => Some("Stopped"),
        AssistantStatus::Interrupted => Some("Interrupted"),
        AssistantStatus::Streaming | AssistantStatus::Complete => None,
    }
}

#[derive(Default)]
pub struct ChatState {
    rows: Vec<IndexRow>,
    pub open: Option<Session>,
    runs: HashMap<SessionId, (TurnId, RunBadge)>,
    /// Model steps taken so far in each session's current run.
    steps: HashMap<SessionId, u32>,
    /// Sessions with a run in progress. A run spans several model turns for
    /// an agent, so this outlives `runs`, which tracks only the streaming turn.
    active: HashSet<SessionId>,
    /// Steering messages sent to a running agent, waiting for its next safe step.
    queued: HashMap<SessionId, Vec<String>>,
    /// Whether each model advertises tool support (from `/api/show`).
    tool_support: HashMap<String, bool>,
    drafts: HashMap<SessionId, String>,
    sending: HashSet<SessionId>,
    /// Sessions dropped since the view last looked; it clears their editors.
    forgotten: Vec<SessionId>,
    omitted: HashMap<SessionId, usize>,
    /// Set when the history cannot be used at all.
    pub unavailable: Option<String>,
    /// The last rejection or error, shown near the composer.
    pub notice: Option<String>,
    pub warnings: Vec<String>,
    /// The model a session wanted that is not installed.
    pub model_missing: Option<(SessionId, String)>,
    /// True once the manager has sent the first list of sessions.
    pub loaded: bool,
}

impl ChatState {
    pub fn rows(&self) -> &[IndexRow] {
        &self.rows
    }

    pub fn fold(&mut self, ev: ChatEvent) {
        match ev {
            ChatEvent::Rows(rows) => {
                // Sessions that left the list (deleted, incognito closed or
                // cleared) take their draft and open transcript with them.
                // Only sessions the list used to show count as gone: a new chat
                // whose first save failed was never listed, and must survive.
                let present: HashSet<SessionId> = rows.iter().map(|r| r.id).collect();
                let known: HashSet<SessionId> = self.rows.iter().map(|r| r.id).collect();
                let gone: Vec<SessionId> = self
                    .drafts
                    .keys()
                    .copied()
                    .chain(self.open.as_ref().map(|s| s.id))
                    .chain(self.runs.keys().copied())
                    .chain(self.active.iter().copied())
                    .filter(|id| known.contains(id) && !present.contains(id))
                    .collect();
                for id in gone {
                    self.forget(id);
                }
                if let Some((id, model)) = &self.model_missing {
                    let changed = rows.iter().any(|r| r.id == *id && r.model != *model);
                    if changed || !present.contains(id) && known.contains(id) {
                        self.model_missing = None;
                    }
                }
                self.rows = rows;
                self.loaded = true;
            }
            ChatEvent::Opened(s) => {
                self.notice = None;
                self.open = Some(*s);
            }
            ChatEvent::Appended { session, message } => self.appended(session, *message),
            ChatEvent::Removed { session, message } => {
                if let Some(s) = self.open.as_mut().filter(|s| s.id == session) {
                    s.messages.retain(|m| m.id != message);
                }
            }
            ChatEvent::TurnStarted { session, turn } => {
                if self
                    .model_missing
                    .as_ref()
                    .is_some_and(|(id, _)| *id == session)
                {
                    self.model_missing = None;
                }
                self.runs.insert(session, (turn, RunBadge::Running));
                if self.active.insert(session) {
                    self.steps.remove(&session);
                }
                *self.steps.entry(session).or_insert(0) += 1;
                self.omitted.remove(&session);
                self.sending.remove(&session);
                self.notice = None;
            }
            ChatEvent::WaitingForModel { session, turn } => {
                if self.runs.get(&session).is_some_and(|(t, _)| *t == turn) {
                    self.runs.insert(session, (turn, RunBadge::Waiting));
                }
            }
            ChatEvent::Delta {
                session,
                turn,
                content,
                thinking,
            } => {
                if let Some(run) = self.runs.get_mut(&session) {
                    if run.0 == turn {
                        run.1 = RunBadge::Running;
                    }
                }
                if let Some(s) = self.open.as_mut().filter(|s| s.id == session) {
                    if let Some(id) = assistant_of_turn(s, turn) {
                        s.append_assistant(id, &content, &thinking);
                    }
                }
            }
            ChatEvent::ContextTrimmed { session, omitted } => {
                self.omitted.insert(session, omitted);
            }
            ChatEvent::TurnEnded {
                session,
                turn,
                outcome,
                message,
                ..
            } => self.turn_ended(session, turn, outcome, message),
            ChatEvent::SendRejected { session, reason } => {
                self.sending.remove(&session);
                self.notice = match reason {
                    SendRejection::Busy => Some("Wait for the current reply to finish.".into()),
                    SendRejection::TooLarge(m) => Some(m),
                    SendRejection::UnknownSession => {
                        Some("That chat is no longer available.".into())
                    }
                    SendRejection::EmptyMessage => None,
                    SendRejection::NoToolSupport(m) => Some(format!(
                        "{m} does not support tools, so it cannot run an agent. Choose a model that does."
                    )),
                    SendRejection::NoWorkspace(why) => {
                        Some(format!("This agent's folder cannot be used: {why}"))
                    }
                    SendRejection::Cancelled => None,
                };
            }
            ChatEvent::ToolStarted {
                session, call_id, ..
            } => {
                self.update_tool(session, &call_id, None, ToolStatus::Running);
            }
            ChatEvent::ToolFinished {
                session,
                call_id,
                ok,
                output,
            } => {
                let status = if ok {
                    ToolStatus::Complete
                } else {
                    ToolStatus::Failed
                };
                self.update_tool(session, &call_id, Some(output), status);
            }
            ChatEvent::SteeringQueued { session, text } => {
                self.drafts.remove(&session);
                self.sending.remove(&session);
                self.queued.entry(session).or_default().push(text);
            }
            ChatEvent::RunEnded {
                session,
                outcome,
                message,
            } => {
                self.active.remove(&session);
                self.runs.remove(&session);
                self.steps.remove(&session);
                self.sending.remove(&session);
                self.queued.remove(&session);
                if outcome == RunOutcome::BudgetExhausted {
                    self.notice = message;
                }
            }
            ChatEvent::AgentSettings {
                session,
                workspace,
                step_budget,
                tools,
            } => {
                if let Some(s) = self.open.as_mut().filter(|s| s.id == session) {
                    s.workspace = workspace;
                    s.step_budget = step_budget;
                    s.effective_tools = tools;
                }
            }
            ChatEvent::ModelCapabilities { model, tools } => {
                self.tool_support.insert(model, tools);
            }
            ChatEvent::ModelMissing { session, model } => {
                self.model_missing = Some((session, model));
            }
            ChatEvent::StoreWarning(m) => {
                self.warnings.push(m);
                if self.warnings.len() > MAX_WARNINGS {
                    self.warnings.remove(0);
                }
            }
            ChatEvent::SessionError { session, message } => {
                if let Some(id) = session {
                    self.sending.remove(&id);
                }
                self.notice = Some(message);
            }
        }
    }

    fn appended(&mut self, session: SessionId, message: Message) {
        if let MessageContent::User { text, delivery } = &message.content {
            if *delivery == UserDelivery::Steering {
                if let Some(q) = self.queued.get_mut(&session) {
                    if let Some(at) = q.iter().position(|t| t == text) {
                        q.remove(at);
                    }
                }
            } else {
                self.drafts.remove(&session);
                self.sending.remove(&session);
            }
        }
        if let Some(s) = self.open.as_mut().filter(|s| s.id == session) {
            s.messages.push(message);
        }
    }

    fn turn_ended(
        &mut self,
        session: SessionId,
        turn: TurnId,
        outcome: TurnOutcome,
        message: Option<String>,
    ) {
        // A duplicate or late terminal event finds no matching run.
        if self.runs.get(&session).map(|(t, _)| *t) != Some(turn) {
            return;
        }
        self.runs.remove(&session);
        self.sending.remove(&session);
        if let Some(s) = self.open.as_mut().filter(|s| s.id == session) {
            if let Some(id) = assistant_of_turn(s, turn) {
                let status = match outcome {
                    TurnOutcome::Completed => AssistantStatus::Complete,
                    TurnOutcome::Failed => AssistantStatus::Failed,
                    TurnOutcome::Cancelled => AssistantStatus::Cancelled,
                    TurnOutcome::Interrupted => AssistantStatus::Interrupted,
                };
                let now = s.updated_at_ms;
                s.finish_assistant(id, status, message, now);
            }
        }
    }

    pub fn badge(&self, id: SessionId) -> RunBadge {
        match self.runs.get(&id) {
            Some(r) => r.1,
            // Between steps (a tool is running) the session is still busy.
            None if self.active.contains(&id) => RunBadge::Running,
            None => RunBadge::None,
        }
    }

    pub fn is_running(&self, id: SessionId) -> bool {
        self.active.contains(&id) || self.runs.contains_key(&id) || self.sending.contains(&id)
    }

    /// Sessions with a run in progress, shown or not.
    #[cfg(test)]
    pub fn active_runs(&self) -> usize {
        self.active.len()
    }

    /// Runs in progress for one kind of session (a tab's badge).
    pub fn active_runs_for(&self, mode: SessionMode) -> usize {
        self.active
            .iter()
            .filter(|id| self.is_agent(**id) == (mode == SessionMode::Agent))
            .count()
    }

    /// Whether the saved or open session is an agent.
    pub fn is_agent(&self, id: SessionId) -> bool {
        self.rows
            .iter()
            .find(|r| r.id == id)
            .map(|r| r.mode == SessionMode::Agent)
            .or_else(|| {
                self.open
                    .as_ref()
                    .filter(|s| s.id == id)
                    .map(|s| s.mode == SessionMode::Agent)
            })
            .unwrap_or(false)
    }

    /// The step a running agent is on (1-based); 0 when it is not running.
    pub fn step_of(&self, id: SessionId) -> u32 {
        self.steps.get(&id).copied().unwrap_or(0)
    }

    /// Steering messages still waiting to be applied to a running agent.
    pub fn queued(&self, id: SessionId) -> &[String] {
        self.queued.get(&id).map_or(&[], Vec::as_slice)
    }

    /// `Some(true)` once the model is known to support tools.
    pub fn supports_tools(&self, model: &str) -> Option<bool> {
        self.tool_support.get(model).copied()
    }

    fn update_tool(
        &mut self,
        id: SessionId,
        call_id: &str,
        result: Option<String>,
        new: ToolStatus,
    ) {
        let Some(s) = self.open.as_mut().filter(|s| s.id == id) else {
            return;
        };
        for m in &mut s.messages {
            if let MessageContent::Tool {
                call_id: c,
                output,
                status,
                ..
            } = &mut m.content
            {
                if c == call_id {
                    *status = new;
                    if result.is_some() {
                        *output = result;
                    }
                    return;
                }
            }
        }
    }

    pub fn omitted(&self, id: SessionId) -> usize {
        self.omitted.get(&id).copied().unwrap_or(0)
    }

    pub fn draft(&self, id: SessionId) -> &str {
        self.drafts.get(&id).map_or("", String::as_str)
    }

    pub fn draft_mut(&mut self, id: SessionId) -> &mut String {
        self.drafts.entry(id).or_default()
    }

    /// Forgets a session's composer text (used when an incognito session ends).
    pub fn forget(&mut self, id: SessionId) {
        self.drafts.remove(&id);
        self.omitted.remove(&id);
        self.runs.remove(&id);
        self.active.remove(&id);
        self.queued.remove(&id);
        self.sending.remove(&id);
        self.forgotten.push(id);
        if self.open.as_ref().is_some_and(|s| s.id == id) {
            self.open = None;
        }
    }

    /// Sessions forgotten since the last call (to drop their editor state).
    pub fn take_forgotten(&mut self) -> Vec<SessionId> {
        std::mem::take(&mut self.forgotten)
    }

    /// Enter / Send. The draft stays until the manager confirms the message.
    pub fn try_send(&mut self, id: SessionId) -> SendAttempt {
        // A running agent takes the message as steering; a chat does not.
        let steering = self.active.contains(&id) && self.is_agent(id);
        if self.sending.contains(&id) || (self.is_running(id) && !steering) {
            return SendAttempt::Busy;
        }
        let text = self.draft(id).trim().to_string();
        if text.is_empty() {
            return SendAttempt::Empty;
        }
        self.sending.insert(id);
        SendAttempt::Sent(ChatCommand::Send { session: id, text })
    }

    /// Marks a retry as in flight so a second press does nothing.
    pub fn try_retry(&mut self, id: SessionId) -> SendAttempt {
        if self.is_running(id) {
            return SendAttempt::Busy;
        }
        self.sending.insert(id);
        SendAttempt::Sent(ChatCommand::Retry(id))
    }

    /// Marks a regenerate as in flight, like `try_retry`.
    pub fn try_regenerate(&mut self, id: SessionId) -> SendAttempt {
        if self.is_running(id) {
            return SendAttempt::Busy;
        }
        self.sending.insert(id);
        SendAttempt::Sent(ChatCommand::Regenerate(id))
    }

    /// The model for a new chat: the newest saved session's, if installed,
    /// else the first installed model alphabetically.
    pub fn default_model(&self, installed: &[String]) -> Option<String> {
        let recent = self
            .rows
            .iter()
            .filter(|r| r.lifetime == SessionLifetime::Persisted && !r.model.is_empty())
            .map(|r| r.model.clone())
            .next();
        if let Some(m) = recent {
            if installed.is_empty() || installed.contains(&m) {
                return Some(m);
            }
        }
        installed.iter().min().cloned()
    }

    /// Groups rows for the rail. `day_of` maps a timestamp to a day number.
    #[cfg(test)]
    pub fn rail(&self, now_ms: u64, day_of: impl Fn(u64) -> i64) -> Rail {
        self.rail_for(SessionMode::Chat, "", now_ms, day_of)
    }

    /// The rail for one tab: chats for Chat, agents for Work. A non-empty
    /// `query` keeps the sessions whose title, first message or model
    /// contains every word of it, ignoring case.
    pub fn rail_for(
        &self,
        mode: SessionMode,
        query: &str,
        now_ms: u64,
        day_of: impl Fn(u64) -> i64,
    ) -> Rail {
        let today = day_of(now_ms);
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        let matches = |r: &IndexRow| {
            let hay = format!(
                "{}\n{}\n{}",
                display_title(r),
                r.preview.as_deref().unwrap_or(""),
                r.model
            )
            .to_lowercase();
            words.iter().all(|w| hay.contains(w))
        };
        let mut rail = Rail::default();
        let mut sorted: Vec<&IndexRow> = self
            .rows
            .iter()
            .filter(|r| r.mode == mode && matches(r))
            .collect();
        sorted.sort_by(|a, b| {
            b.updated_at_ms
                .cmp(&a.updated_at_ms)
                .then_with(|| a.id.cmp(&b.id))
        });
        for r in sorted {
            let row = RailRow {
                id: r.id,
                title: display_title(r),
                model: r.model.clone(),
                pinned: r.pinned,
                badge: self.badge(r.id),
                status: r.status,
                lifetime: r.lifetime,
            };
            let group = if r.lifetime == SessionLifetime::Incognito {
                &mut rail.incognito
            } else if r.pinned {
                &mut rail.pinned
            } else {
                match today - day_of(r.updated_at_ms) {
                    d if d <= 0 => &mut rail.today,
                    1 => &mut rail.yesterday,
                    _ => &mut rail.earlier,
                }
            };
            group.push(row);
        }
        rail
    }

    pub fn dismiss_warnings(&mut self) {
        self.warnings.clear();
    }

    pub fn recorded_unavailable(&mut self, why: String) {
        self.unavailable = Some(why);
    }
}

fn assistant_of_turn(s: &Session, turn: TurnId) -> Option<MessageId> {
    s.messages.iter().rev().find_map(|m| match &m.content {
        MessageContent::Assistant {
            turn_id: Some(t), ..
        } if *t == turn => Some(m.id),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use roost_core::session::RunId;

    const DAY: u64 = DAY_MS;
    const NOW: u64 = 10 * DAY + 12 * 3_600_000;

    fn utc_day(ms: u64) -> i64 {
        (ms / DAY) as i64
    }

    fn row(title: &str, updated: u64) -> IndexRow {
        let s = Session::new_chat("m", SessionLifetime::Persisted, updated);
        let mut r = roost_core_row(&s);
        r.title = title.into();
        r
    }

    fn roost_core_row(s: &Session) -> IndexRow {
        IndexRow {
            id: s.id,
            title: s.title.clone(),
            mode: s.mode,
            lifetime: s.lifetime,
            pinned: s.pinned,
            model: s.model.clone(),
            parent: None,
            created_at_ms: s.created_at_ms,
            updated_at_ms: s.updated_at_ms,
            active: false,
            status: RowStatus::Ok,
            file_mtime_ms: 0,
            file_len: 0,
            preview: Some(String::new()),
        }
    }

    fn started() -> (ChatState, Session, TurnId) {
        let mut st = ChatState::default();
        let mut s = Session::new_chat("m", SessionLifetime::Persisted, 1);
        s.push_user("hi", 2);
        let turn = TurnId::new();
        let a = s.begin_assistant(RunId::new(), turn, 3);
        let assistant = s.messages.iter().find(|m| m.id == a).cloned().unwrap();
        let mut shown = s.clone();
        shown.messages.truncate(1);
        st.fold(ChatEvent::Opened(Box::new(shown)));
        st.fold(ChatEvent::Appended {
            session: s.id,
            message: Box::new(assistant),
        });
        st.fold(ChatEvent::TurnStarted {
            session: s.id,
            turn,
        });
        (st, s, turn)
    }

    fn last_text(st: &ChatState) -> (String, AssistantStatus) {
        match &st.open.as_ref().unwrap().messages.last().unwrap().content {
            MessageContent::Assistant { text, status, .. } => (text.clone(), *status),
            _ => panic!("not an assistant message"),
        }
    }

    fn delta(s: &Session, turn: TurnId, c: &str) -> ChatEvent {
        ChatEvent::Delta {
            session: s.id,
            turn,
            content: c.into(),
            thinking: String::new(),
        }
    }

    fn ended(s: &Session, turn: TurnId, outcome: TurnOutcome) -> ChatEvent {
        ChatEvent::TurnEnded {
            session: s.id,
            turn,
            outcome,
            message: None,
            prompt_eval_count: None,
        }
    }

    #[test]
    fn deltas_fold_into_the_streaming_message_by_turn() {
        let (mut st, s, turn) = started();
        st.fold(delta(&s, turn, "Hel"));
        st.fold(delta(&s, turn, "lo"));
        st.fold(delta(&s, TurnId::new(), "stray"));
        assert_eq!(last_text(&st), ("Hello".into(), AssistantStatus::Streaming));
        st.fold(ended(&s, turn, TurnOutcome::Completed));
        assert_eq!(last_text(&st), ("Hello".into(), AssistantStatus::Complete));
        assert_eq!(st.active_runs(), 1, "the run lasts until RunEnded");
        st.fold(ChatEvent::RunEnded {
            session: s.id,
            outcome: RunOutcome::Completed,
            message: None,
        });
        assert_eq!(st.active_runs(), 0);
    }

    #[test]
    fn a_duplicate_or_late_terminal_event_is_ignored() {
        let (mut st, s, turn) = started();
        st.fold(delta(&s, turn, "x"));
        st.fold(ended(&s, turn, TurnOutcome::Completed));
        st.fold(ended(&s, turn, TurnOutcome::Failed));
        assert_eq!(last_text(&st).1, AssistantStatus::Complete);
    }

    #[test]
    fn terminal_labels_map_correctly() {
        assert_eq!(status_label(AssistantStatus::Failed), Some("Failed"));
        assert_eq!(status_label(AssistantStatus::Cancelled), Some("Stopped"));
        assert_eq!(
            status_label(AssistantStatus::Interrupted),
            Some("Interrupted")
        );
        assert_eq!(status_label(AssistantStatus::Complete), None);
        assert_eq!(status_label(AssistantStatus::Streaming), None);
    }

    #[test]
    fn a_failure_keeps_partial_text_and_records_the_message() {
        let (mut st, s, turn) = started();
        st.fold(delta(&s, turn, "half"));
        st.fold(ChatEvent::TurnEnded {
            session: s.id,
            turn,
            outcome: TurnOutcome::Failed,
            message: Some("boom".into()),
            prompt_eval_count: None,
        });
        assert_eq!(last_text(&st), ("half".into(), AssistantStatus::Failed));
        let MessageContent::Assistant { error, .. } =
            &st.open.as_ref().unwrap().messages.last().unwrap().content
        else {
            panic!()
        };
        assert_eq!(error.as_deref(), Some("boom"));
    }

    #[test]
    fn a_pinned_session_is_listed_once_under_pinned() {
        let mut st = ChatState::default();
        let mut r = row("p", NOW);
        r.pinned = true;
        st.fold(ChatEvent::Rows(vec![r.clone(), row("q", NOW)]));
        let rail = st.rail(NOW, utc_day);
        assert_eq!(rail.pinned.len(), 1);
        assert_eq!(rail.today.len(), 1);
        assert_eq!(rail.pinned[0].id, r.id);
    }

    #[test]
    fn rail_groups_today_yesterday_earlier_and_incognito() {
        let mut st = ChatState::default();
        let mut inc = row("i", NOW);
        inc.lifetime = SessionLifetime::Incognito;
        st.fold(ChatEvent::Rows(vec![
            row("t", NOW),
            row("y", NOW - DAY),
            row("e", NOW - 5 * DAY),
            inc,
        ]));
        let rail = st.rail(NOW, utc_day);
        let titles = |v: &Vec<RailRow>| v.iter().map(|r| r.title.clone()).collect::<Vec<_>>();
        assert_eq!(titles(&rail.today), ["t"]);
        assert_eq!(titles(&rail.yesterday), ["y"]);
        assert_eq!(titles(&rail.earlier), ["e"]);
        assert_eq!(titles(&rail.incognito), ["i"]);
    }

    #[test]
    fn rows_stay_in_activity_order_when_only_a_title_changes() {
        let mut st = ChatState::default();
        let (a, b) = (row("a", NOW), row("b", NOW - 1000));
        st.fold(ChatEvent::Rows(vec![a.clone(), b.clone()]));
        let before: Vec<_> = st.rail(NOW, utc_day).today.iter().map(|r| r.id).collect();
        let mut renamed = b.clone();
        renamed.title = "renamed".into();
        st.fold(ChatEvent::Rows(vec![a, renamed]));
        let after: Vec<_> = st.rail(NOW, utc_day).today.iter().map(|r| r.id).collect();
        assert_eq!(before, after);
    }

    #[test]
    fn active_run_count_covers_background_sessions() {
        let (mut st, s, turn) = started();
        assert_eq!(st.active_runs(), 1);
        assert_eq!(st.badge(s.id), RunBadge::Running);
        st.fold(ChatEvent::WaitingForModel {
            session: s.id,
            turn,
        });
        assert_eq!(st.badge(s.id), RunBadge::Waiting);
        st.fold(delta(&s, turn, "x"));
        assert_eq!(st.badge(s.id), RunBadge::Running);
        st.open = None; // viewing something else does not end the run
        assert_eq!(st.active_runs(), 1);
    }

    #[test]
    fn default_model_prefers_the_latest_session_then_alphabetical() {
        let mut st = ChatState::default();
        let installed = vec!["zeta:1".to_string(), "alpha:1".to_string()];
        assert_eq!(st.default_model(&installed), Some("alpha:1".into()));
        let mut r = row("a", NOW);
        r.model = "zeta:1".into();
        st.fold(ChatEvent::Rows(vec![r.clone()]));
        assert_eq!(st.default_model(&installed), Some("zeta:1".into()));
        r.model = "removed:1".into();
        st.fold(ChatEvent::Rows(vec![r]));
        assert_eq!(st.default_model(&installed), Some("alpha:1".into()));
        assert_eq!(ChatState::default().default_model(&[]), None);
    }

    #[test]
    fn enter_during_a_run_keeps_the_draft() {
        let (mut st, s, _turn) = started();
        *st.draft_mut(s.id) = "next question".into();
        assert!(matches!(st.try_send(s.id), SendAttempt::Busy));
        assert_eq!(st.draft(s.id), "next question");
    }

    #[test]
    fn the_draft_is_cleared_only_when_the_message_is_confirmed() {
        let mut st = ChatState::default();
        let s = Session::new_chat("m", SessionLifetime::Persisted, 1);
        st.fold(ChatEvent::Opened(Box::new(s.clone())));
        *st.draft_mut(s.id) = "hello".into();
        let SendAttempt::Sent(_) = st.try_send(s.id) else {
            panic!("expected a send");
        };
        assert_eq!(st.draft(s.id), "hello", "kept until confirmed");
        assert!(
            matches!(st.try_send(s.id), SendAttempt::Busy),
            "double press"
        );
        st.fold(ChatEvent::SendRejected {
            session: s.id,
            reason: SendRejection::TooLarge("too big".into()),
        });
        assert_eq!(st.draft(s.id), "hello");
        assert_eq!(st.notice.as_deref(), Some("too big"));
        assert!(matches!(st.try_send(s.id), SendAttempt::Sent(_)));
    }

    #[test]
    fn empty_drafts_are_not_sent() {
        let mut st = ChatState::default();
        let s = Session::new_chat("m", SessionLifetime::Persisted, 1);
        *st.draft_mut(s.id) = "   ".into();
        assert!(matches!(st.try_send(s.id), SendAttempt::Empty));
    }

    #[test]
    fn unavailable_history_is_recorded() {
        let mut st = ChatState::default();
        st.recorded_unavailable("Another Roost is using this history".into());
        assert!(st.unavailable.as_deref().unwrap().contains("Another Roost"));
    }

    #[test]
    fn rows_without_a_session_drop_its_draft_open_transcript_and_run() {
        let (mut st, s, _turn) = started();
        st.fold(ChatEvent::Rows(vec![roost_core_row(&s)]));
        *st.draft_mut(s.id) = "secret".into();
        st.fold(ChatEvent::Rows(vec![]));
        assert_eq!(st.draft(s.id), "");
        assert!(st.open.is_none());
        assert_eq!(st.active_runs(), 0);
    }

    #[test]
    fn forgetting_a_session_drops_its_draft_and_transcript() {
        let (mut st, s, _t) = started();
        *st.draft_mut(s.id) = "secret".into();
        st.forget(s.id);
        assert_eq!(st.draft(s.id), "");
        assert!(st.open.is_none());
    }

    #[test]
    fn a_session_the_list_never_contained_is_not_discarded() {
        // A failed first save means the list may never show a new chat.
        let mut st = ChatState::default();
        let s = Session::new_chat("m", SessionLifetime::Persisted, 1);
        st.fold(ChatEvent::Opened(Box::new(s.clone())));
        *st.draft_mut(s.id) = "typed".into();
        st.fold(ChatEvent::Rows(vec![]));
        assert!(st.open.is_some());
        assert_eq!(st.draft(s.id), "typed");
    }

    #[test]
    fn a_rejected_retry_does_not_leave_the_session_stuck() {
        let mut st = ChatState::default();
        let s = Session::new_chat("m", SessionLifetime::Persisted, 1);
        st.fold(ChatEvent::Opened(Box::new(s.clone())));
        assert!(matches!(st.try_retry(s.id), SendAttempt::Sent(_)));
        assert!(st.is_running(s.id));
        st.fold(ChatEvent::SessionError {
            session: Some(s.id),
            message: "There is nothing to retry.".into(),
        });
        assert!(!st.is_running(s.id));
        assert!(matches!(st.try_retry(s.id), SendAttempt::Sent(_)));
    }

    #[test]
    fn the_missing_model_banner_clears_when_the_model_changes_or_a_turn_starts() {
        let mut st = ChatState::default();
        let s = Session::new_chat("gone:1", SessionLifetime::Persisted, 1);
        let mut r = roost_core_row(&s);
        st.fold(ChatEvent::Rows(vec![r.clone()]));
        st.fold(ChatEvent::ModelMissing {
            session: s.id,
            model: "gone:1".into(),
        });
        assert!(st.model_missing.is_some());
        r.model = "new:2".into();
        st.fold(ChatEvent::Rows(vec![r]));
        assert!(st.model_missing.is_none(), "cleared by a model change");
        st.fold(ChatEvent::ModelMissing {
            session: s.id,
            model: "new:2".into(),
        });
        st.fold(ChatEvent::TurnStarted {
            session: s.id,
            turn: TurnId::new(),
        });
        assert!(st.model_missing.is_none(), "cleared when a turn starts");
    }

    #[test]
    fn warnings_are_capped_and_can_be_dismissed() {
        let mut st = ChatState::default();
        for i in 0..20 {
            st.fold(ChatEvent::StoreWarning(format!("w{i}")));
        }
        assert_eq!(st.warnings.len(), 5);
        assert_eq!(st.warnings.last().unwrap(), "w19");
        st.dismiss_warnings();
        assert!(st.warnings.is_empty());
    }

    #[test]
    fn an_untitled_chat_shows_the_start_of_its_first_message() {
        let mut st = ChatState::default();
        let mut r = row("Untitled", NOW);
        r.preview = Some("How do I make sourdough?".into());
        let mut empty = row("Untitled", NOW - 1000);
        empty.preview = Some(String::new());
        let mut named = row("Bread notes", NOW - 2000);
        named.preview = Some("ignored preview".into());
        let mut unknown = row("Untitled", NOW - 3000);
        unknown.preview = None;
        st.fold(ChatEvent::Rows(vec![r, empty, named, unknown]));
        let titles: Vec<String> = st
            .rail(NOW, utc_day)
            .today
            .iter()
            .map(|r| r.title.clone())
            .collect();
        assert_eq!(
            titles,
            [
                "How do I make sourdough?",
                "Untitled",
                "Bread notes",
                "Untitled"
            ]
        );
    }

    #[test]
    fn display_title_prefers_a_real_title() {
        let mut r = row("Untitled", NOW);
        r.preview = Some("hello".into());
        assert_eq!(display_title(&r), "hello");
        r.title = "Mine".into();
        assert_eq!(display_title(&r), "Mine");
    }

    // ---- agents

    fn agent_state() -> (ChatState, Session) {
        let mut st = ChatState::default();
        let mut s =
            Session::new_agent("m", SessionLifetime::Persisted, std::env::temp_dir(), 1).unwrap();
        s.push_user("task", 2);
        let mut r = roost_core_row(&s);
        r.mode = SessionMode::Agent;
        st.fold(ChatEvent::Rows(vec![r]));
        st.fold(ChatEvent::Opened(Box::new(s.clone())));
        (st, s)
    }

    fn run_ended(s: &Session, outcome: RunOutcome, message: Option<&str>) -> ChatEvent {
        ChatEvent::RunEnded {
            session: s.id,
            outcome,
            message: message.map(str::to_string),
        }
    }

    #[test]
    fn an_agent_run_stays_active_between_model_turns_until_the_run_ends() {
        let (mut st, s) = agent_state();
        let turn = TurnId::new();
        st.fold(ChatEvent::TurnStarted {
            session: s.id,
            turn,
        });
        st.fold(ended(&s, turn, TurnOutcome::Completed));
        // A tool is running now: no turn is streaming, but the agent is busy.
        assert_eq!(st.active_runs(), 1);
        assert!(st.is_running(s.id));
        assert_eq!(st.badge(s.id), RunBadge::Running);
        st.fold(ChatEvent::TurnStarted {
            session: s.id,
            turn: TurnId::new(),
        });
        assert_eq!(st.active_runs(), 1, "still one run");
        st.fold(run_ended(&s, RunOutcome::Completed, None));
        assert!(!st.is_running(s.id));
        assert_eq!(st.badge(s.id), RunBadge::None);
    }

    #[test]
    fn tool_events_update_the_tool_step_in_the_open_transcript() {
        let (mut st, s) = agent_state();
        let mut with_tool = s.clone();
        let id = with_tool.push_tool_call(
            roost_core::session::RunId::new(),
            "call_1",
            "read_file",
            serde_json::json!({"path": "a"}),
            3,
        );
        let msg = with_tool
            .messages
            .iter()
            .find(|m| m.id == id)
            .cloned()
            .unwrap();
        st.fold(ChatEvent::Appended {
            session: s.id,
            message: Box::new(msg),
        });
        let tool =
            |st: &ChatState| match &st.open.as_ref().unwrap().messages.last().unwrap().content {
                MessageContent::Tool { status, output, .. } => (*status, output.clone()),
                _ => panic!("not a tool message"),
            };
        assert_eq!(tool(&st), (ToolStatus::Pending, None));
        st.fold(ChatEvent::ToolStarted {
            session: s.id,
            call_id: "call_1".into(),
            name: "read_file".into(),
            arguments: serde_json::json!({}),
        });
        assert_eq!(tool(&st).0, ToolStatus::Running);
        st.fold(ChatEvent::ToolFinished {
            session: s.id,
            call_id: "call_1".into(),
            ok: true,
            output: "1| hi".into(),
        });
        assert_eq!(tool(&st), (ToolStatus::Complete, Some("1| hi".into())));
        st.fold(ChatEvent::ToolFinished {
            session: s.id,
            call_id: "unknown".into(),
            ok: false,
            output: "x".into(),
        });
        assert_eq!(
            tool(&st).0,
            ToolStatus::Complete,
            "an unknown call changes nothing"
        );
    }

    #[test]
    fn an_agent_accepts_steering_while_running_and_a_chat_does_not() {
        let (mut st, s) = agent_state();
        st.fold(ChatEvent::TurnStarted {
            session: s.id,
            turn: TurnId::new(),
        });
        *st.draft_mut(s.id) = "look at src instead".into();
        let SendAttempt::Sent(ChatCommand::Send { text, .. }) = st.try_send(s.id) else {
            panic!("an agent should take steering")
        };
        assert_eq!(text, "look at src instead");
        assert!(
            matches!(st.try_send(s.id), SendAttempt::Busy),
            "double press while in flight"
        );
        st.fold(ChatEvent::SteeringQueued {
            session: s.id,
            text,
        });
        assert_eq!(st.queued(s.id), ["look at src instead"]);
        assert_eq!(st.draft(s.id), "", "the draft is cleared once queued");
        // Applying it moves it into the transcript and out of the queue.
        let mut after = s.clone();
        let id = after.push_steering("look at src instead", 5);
        let msg = after.messages.iter().find(|m| m.id == id).cloned().unwrap();
        st.fold(ChatEvent::Appended {
            session: s.id,
            message: Box::new(msg),
        });
        assert!(st.queued(s.id).is_empty());

        // A chat still refuses to send while it replies.
        let (mut chat, cs, _t) = started();
        *chat.draft_mut(cs.id) = "next".into();
        assert!(matches!(chat.try_send(cs.id), SendAttempt::Busy));
    }

    #[test]
    fn the_rail_for_each_tab_lists_only_its_own_kind_of_session() {
        let mut st = ChatState::default();
        let mut agent = row("Refactor plan", NOW);
        agent.mode = SessionMode::Agent;
        st.fold(ChatEvent::Rows(vec![row("A chat", NOW), agent]));
        let titles = |mode| {
            st.rail_for(mode, "", NOW, utc_day)
                .today
                .iter()
                .map(|r| r.title.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(titles(SessionMode::Chat), ["A chat"]);
        assert_eq!(titles(SessionMode::Agent), ["Refactor plan"]);
        assert_eq!(st.rail(NOW, utc_day).today.len(), 1);
    }

    #[test]
    fn the_rail_search_matches_every_word_in_title_first_message_or_model() {
        let mut st = ChatState::default();
        let mut a = row("Trip planning", NOW);
        a.model = "llama3.2:3b".into();
        let mut b = row("Untitled", NOW - 1);
        b.preview = Some("Explain borrow checker errors".into());
        let c = row("Recipes", NOW - 2);
        st.fold(ChatEvent::Rows(vec![a, b, c]));
        let titles = |q: &str| {
            st.rail_for(SessionMode::Chat, q, NOW, utc_day)
                .today
                .iter()
                .map(|r| r.title.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(titles("").len(), 3);
        assert_eq!(titles("TRIP"), ["Trip planning"]);
        assert_eq!(titles("borrow errors"), ["Explain borrow checker errors"]);
        assert_eq!(titles("llama3.2"), ["Trip planning"]);
        assert!(titles("trip borrow").is_empty(), "every word must match");
        assert!(titles("zzz").is_empty());
    }

    #[test]
    fn a_removed_message_leaves_the_open_chat() {
        let (mut st, s, turn) = started();
        let old = st.open.as_ref().unwrap().messages.last().unwrap().id;
        st.fold(ChatEvent::TurnEnded {
            session: s.id,
            turn,
            outcome: TurnOutcome::Completed,
            message: None,
            prompt_eval_count: None,
        });
        st.fold(ChatEvent::Removed {
            session: s.id,
            message: old,
        });
        assert!(st
            .open
            .as_ref()
            .unwrap()
            .messages
            .iter()
            .all(|m| m.id != old));
    }

    #[test]
    fn tool_support_is_remembered_per_model_and_refusals_are_explained() {
        let mut st = ChatState::default();
        assert_eq!(st.supports_tools("m:1"), None);
        st.fold(ChatEvent::ModelCapabilities {
            model: "m:1".into(),
            tools: true,
        });
        st.fold(ChatEvent::ModelCapabilities {
            model: "n:2".into(),
            tools: false,
        });
        assert_eq!(st.supports_tools("m:1"), Some(true));
        assert_eq!(st.supports_tools("n:2"), Some(false));
        let id = Session::new_chat("n:2", SessionLifetime::Persisted, 1).id;
        st.fold(ChatEvent::SendRejected {
            session: id,
            reason: SendRejection::NoToolSupport("n:2".into()),
        });
        assert!(st
            .notice
            .as_deref()
            .unwrap()
            .contains("does not support tools"));
        st.fold(ChatEvent::SendRejected {
            session: id,
            reason: SendRejection::NoWorkspace("gone".into()),
        });
        assert!(st.notice.as_deref().unwrap().contains("folder"));
    }

    #[test]
    fn an_interrupted_turn_is_kept_and_labelled() {
        let (mut st, s, turn) = started();
        st.fold(delta(&s, turn, "partial"));
        st.fold(ended(&s, turn, TurnOutcome::Interrupted));
        assert_eq!(
            last_text(&st),
            ("partial".into(), AssistantStatus::Interrupted)
        );
        assert_eq!(
            status_label(AssistantStatus::Interrupted),
            Some("Interrupted")
        );
    }

    #[test]
    fn running_out_of_steps_is_reported_to_the_user() {
        let (mut st, s) = agent_state();
        st.fold(ChatEvent::TurnStarted {
            session: s.id,
            turn: TurnId::new(),
        });
        st.fold(run_ended(
            &s,
            RunOutcome::BudgetExhausted,
            Some("Stopped after using all 20 steps."),
        ));
        assert_eq!(
            st.notice.as_deref(),
            Some("Stopped after using all 20 steps.")
        );
        assert!(!st.is_running(s.id));
    }
}
