//! Chat view-model: folds `ChatEvent`s into plain data. No egui types.

use roost_core::chat::{ChatCommand, ChatEvent, SendRejection, TurnOutcome};
use roost_core::session::{
    AssistantStatus, Message, MessageContent, MessageId, Session, SessionId, SessionLifetime,
    TurnId,
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
            ChatEvent::TurnStarted { session, turn } => {
                if self
                    .model_missing
                    .as_ref()
                    .is_some_and(|(id, _)| *id == session)
                {
                    self.model_missing = None;
                }
                self.runs.insert(session, (turn, RunBadge::Running));
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
                    SendRejection::Cancelled => None,
                };
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
        if matches!(message.content, MessageContent::User { .. }) {
            self.drafts.remove(&session);
            self.sending.remove(&session);
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
                };
                let now = s.updated_at_ms;
                s.finish_assistant(id, status, message, now);
            }
        }
    }

    pub fn badge(&self, id: SessionId) -> RunBadge {
        self.runs.get(&id).map_or(RunBadge::None, |r| r.1)
    }

    pub fn is_running(&self, id: SessionId) -> bool {
        self.runs.contains_key(&id) || self.sending.contains(&id)
    }

    /// Sessions with a reply in progress, shown or not.
    pub fn active_runs(&self) -> usize {
        self.runs.len()
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
        if self.is_running(id) {
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
    pub fn rail(&self, now_ms: u64, day_of: impl Fn(u64) -> i64) -> Rail {
        let today = day_of(now_ms);
        let mut rail = Rail::default();
        let mut sorted: Vec<&IndexRow> = self.rows.iter().collect();
        sorted.sort_by(|a, b| {
            b.updated_at_ms
                .cmp(&a.updated_at_ms)
                .then_with(|| a.id.cmp(&b.id))
        });
        for r in sorted {
            let row = RailRow {
                id: r.id,
                title: r.title.clone(),
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
}
