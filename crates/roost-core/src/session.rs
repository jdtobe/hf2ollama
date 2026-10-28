//! Stable session domain types shared by chat, agents and the history store.
//!
//! This module only defines data and its validation. Persistence, Ollama
//! calls and agent execution belong to later stages.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::str::FromStr;
use thiserror::Error;
use uuid::Uuid;

/// Version written into every stored session document.
pub const SESSION_SCHEMA_VERSION: u32 = 1;
const DEFAULT_STEP_BUDGET: u32 = 20;
const MAX_TITLE_CHARS: usize = 80;
/// The title every new session starts with.
pub const UNTITLED: &str = "Untitled";

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SessionError {
    #[error("the agent workspace must be an absolute path without `..`: {0}")]
    InvalidWorkspace(String),
    #[error("a title must be 1 to 80 characters")]
    InvalidTitle,
    #[error("no message with that id in this session")]
    UnknownMessage,
    #[error("only a completed assistant response can be branched from")]
    NotABranchBoundary,
    #[error("unsupported session schema version {0}")]
    UnsupportedSchema(u32),
}

macro_rules! id_type {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            #[allow(clippy::new_without_default)]
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.hyphenated().fmt(f)
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Uuid::parse_str(s).map(Self)
            }
        }
    };
}

id_type!(
    /// Names a session; also the stem of its file in the session store.
    SessionId
);
id_type!(
    /// Names one transcript message; stable across branching.
    MessageId
);
id_type!(
    /// Names one agent run (a loop of turns and tool calls).
    RunId
);
id_type!(
    /// Names one model request within a run.
    TurnId
);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SessionMode {
    Chat,
    Agent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SessionLifetime {
    Persisted,
    Incognito,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SystemPromptKind {
    Chat,
    Agent,
}

/// How a user message reached the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UserDelivery {
    Normal,
    /// Sent while an agent was running; applied at its next safe boundary.
    Steering,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AssistantStatus {
    Streaming,
    Complete,
    Interrupted,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolStatus {
    Pending,
    Running,
    Complete,
    Failed,
    Cancelled,
}

/// What the session's agent run is doing; active states carry stable IDs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum RunState {
    Idle,
    WaitingForModel { run_id: RunId, turn_id: TurnId },
    Running { run_id: RunId, turn_id: TurnId },
    Stopping { run_id: RunId },
}

/// Per-session Ollama options; `None` leaves the model's own default.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelOptions {
    pub num_ctx: Option<u32>,
    pub temperature: Option<f32>,
    pub seed: Option<i64>,
    pub think: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TitleMetadata {
    pub auto_title: bool,
    pub last_titled_at_ms: Option<u64>,
    pub undo_title: Option<String>,
}

impl Default for TitleMetadata {
    fn default() -> Self {
        TitleMetadata {
            auto_title: true,
            last_titled_at_ms: None,
            undo_title: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Permission {
    Auto,
    Ask,
    Off,
}

/// A session's tool configuration, independent of any concrete registry:
/// capability permissions and per-tool switches, keyed by registry names.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EffectiveToolConfig {
    pub revision: u32,
    pub capabilities: BTreeMap<String, Permission>,
    pub tools: BTreeMap<String, bool>,
}

/// Where a branch was forked from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchOrigin {
    pub session_id: SessionId,
    pub message_id: MessageId,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "kebab-case")]
pub enum MessageContent {
    User {
        text: String,
        delivery: UserDelivery,
    },
    Assistant {
        text: String,
        status: AssistantStatus,
        run_id: Option<RunId>,
        turn_id: Option<TurnId>,
        /// Readable reason when `status` is `failed`.
        error: Option<String>,
        /// Reasoning text some models return; shown collapsed, never resent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thinking: Option<String>,
        /// What Ollama reported when the reply finished (tokens and timing).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stats: Option<ReplyStats>,
    },
    Tool {
        call_id: String,
        name: String,
        arguments: serde_json::Value,
        output: Option<String>,
        status: ToolStatus,
        run_id: Option<RunId>,
    },
}

/// Numbers Ollama reports with the last message of a reply. Durations are
/// whole milliseconds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplyStats {
    pub prompt_tokens: Option<u64>,
    pub reply_tokens: Option<u64>,
    pub total_ms: Option<u64>,
    pub load_ms: Option<u64>,
    pub eval_ms: Option<u64>,
}

impl ReplyStats {
    /// Reply tokens per second of generation time, when both are known.
    pub fn tokens_per_second(&self) -> Option<f64> {
        let (tokens, ms) = (self.reply_tokens?, self.eval_ms?);
        (ms > 0).then(|| tokens as f64 * 1000.0 / ms as f64)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub id: MessageId,
    pub created_at_ms: u64,
    #[serde(flatten)]
    pub content: MessageContent,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub id: SessionId,
    pub mode: SessionMode,
    pub lifetime: SessionLifetime,
    pub title: String,
    pub title_metadata: TitleMetadata,
    pub pinned: bool,
    pub model: String,
    pub model_options: ModelOptions,
    pub system_prompt: SystemPromptKind,
    pub workspace: Option<PathBuf>,
    pub effective_tools: EffectiveToolConfig,
    pub step_budget: u32,
    pub run_state: RunState,
    pub parent: Option<BranchOrigin>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub messages: Vec<Message>,
}

fn check_workspace(path: &Path) -> Result<(), SessionError> {
    let escapes = path.components().any(|c| matches!(c, Component::ParentDir));
    if !path.is_absolute() || escapes {
        return Err(SessionError::InvalidWorkspace(path.display().to_string()));
    }
    Ok(())
}

impl Session {
    fn base(
        mode: SessionMode,
        model: &str,
        lifetime: SessionLifetime,
        workspace: Option<PathBuf>,
        now_ms: u64,
    ) -> Session {
        Session {
            id: SessionId::new(),
            mode,
            lifetime,
            title: UNTITLED.into(),
            title_metadata: TitleMetadata::default(),
            pinned: false,
            model: model.into(),
            model_options: ModelOptions::default(),
            system_prompt: match mode {
                SessionMode::Chat => SystemPromptKind::Chat,
                SessionMode::Agent => SystemPromptKind::Agent,
            },
            workspace,
            effective_tools: EffectiveToolConfig::default(),
            step_budget: DEFAULT_STEP_BUDGET,
            run_state: RunState::Idle,
            parent: None,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
            messages: Vec::new(),
        }
    }

    pub fn new_chat(model: &str, lifetime: SessionLifetime, now_ms: u64) -> Session {
        Session::base(SessionMode::Chat, model, lifetime, None, now_ms)
    }

    /// An agent session; the workspace must be absolute and contain no `..`.
    pub fn new_agent(
        model: &str,
        lifetime: SessionLifetime,
        workspace: PathBuf,
        now_ms: u64,
    ) -> Result<Session, SessionError> {
        check_workspace(&workspace)?;
        Ok(Session::base(
            SessionMode::Agent,
            model,
            lifetime,
            Some(workspace),
            now_ms,
        ))
    }

    pub fn is_branch(&self) -> bool {
        self.parent.is_some()
    }

    /// True when no run, generation or tool call is in progress.
    pub fn is_idle(&self) -> bool {
        self.run_state == RunState::Idle
            && !self.messages.iter().any(|m| match &m.content {
                MessageContent::User { .. } => false,
                MessageContent::Assistant { status, .. } => *status == AssistantStatus::Streaming,
                MessageContent::Tool { status, .. } => {
                    matches!(status, ToolStatus::Pending | ToolStatus::Running)
                }
            })
    }

    /// Crash recovery for a session loaded with work still marked active:
    /// back to `Idle`, partial replies `Interrupted`, unfinished tool calls
    /// `Cancelled`. Returns whether anything changed.
    pub fn normalize_after_crash(&mut self) -> bool {
        let mut changed = self.run_state != RunState::Idle;
        self.run_state = RunState::Idle;
        for m in &mut self.messages {
            match &mut m.content {
                MessageContent::Assistant { status, .. }
                    if *status == AssistantStatus::Streaming =>
                {
                    *status = AssistantStatus::Interrupted;
                    changed = true;
                }
                MessageContent::Tool { status, .. }
                    if matches!(status, ToolStatus::Pending | ToolStatus::Running) =>
                {
                    *status = ToolStatus::Cancelled;
                    changed = true;
                }
                _ => {}
            }
        }
        changed
    }

    /// The messages up to and including `id`, for a branch. Only a complete
    /// assistant message is a valid boundary.
    pub fn completed_prefix_through(&self, id: MessageId) -> Result<Vec<Message>, SessionError> {
        let at = self
            .messages
            .iter()
            .position(|m| m.id == id)
            .ok_or(SessionError::UnknownMessage)?;
        match &self.messages[at].content {
            MessageContent::Assistant {
                status: AssistantStatus::Complete,
                ..
            } => Ok(self.messages[..=at].to_vec()),
            _ => Err(SessionError::NotABranchBoundary),
        }
    }

    /// Manual rename: trims, remembers the old title for undo and turns
    /// Auto-title off. An invalid title changes nothing.
    pub fn rename(&mut self, title: &str) -> Result<(), SessionError> {
        let title = title.trim();
        if title.is_empty() || title.chars().count() > MAX_TITLE_CHARS {
            return Err(SessionError::InvalidTitle);
        }
        // Keep the earlier undo value when the title does not change.
        if title != self.title {
            self.title_metadata.undo_title = Some(std::mem::replace(&mut self.title, title.into()));
        }
        self.title_metadata.auto_title = false;
        Ok(())
    }

    /// Pinning is housekeeping: it never changes `updated_at_ms`.
    pub fn set_pinned(&mut self, pinned: bool) {
        self.pinned = pinned;
    }

    fn push(&mut self, content: MessageContent, now_ms: u64) -> MessageId {
        let id = MessageId::new();
        self.messages.push(Message {
            id,
            created_at_ms: now_ms,
            content,
        });
        self.updated_at_ms = now_ms;
        id
    }

    pub fn push_user(&mut self, text: &str, now_ms: u64) -> MessageId {
        self.push(
            MessageContent::User {
                text: text.into(),
                delivery: UserDelivery::Normal,
            },
            now_ms,
        )
    }

    /// Starts an empty streaming assistant message and marks the run active.
    pub fn begin_assistant(&mut self, run: RunId, turn: TurnId, now_ms: u64) -> MessageId {
        self.run_state = RunState::Running {
            run_id: run,
            turn_id: turn,
        };
        self.push(
            MessageContent::Assistant {
                text: String::new(),
                status: AssistantStatus::Streaming,
                run_id: Some(run),
                turn_id: Some(turn),
                error: None,
                thinking: None,
                stats: None,
            },
            now_ms,
        )
    }

    /// A message the user sent while an agent was running; it is applied at
    /// the agent's next safe step.
    pub fn push_steering(&mut self, text: &str, now_ms: u64) -> MessageId {
        self.push(
            MessageContent::User {
                text: text.into(),
                delivery: UserDelivery::Steering,
            },
            now_ms,
        )
    }

    /// Records a tool call the model asked for, waiting to run.
    pub fn push_tool_call(
        &mut self,
        run: RunId,
        call_id: &str,
        name: &str,
        arguments: serde_json::Value,
        now_ms: u64,
    ) -> MessageId {
        self.push(
            MessageContent::Tool {
                call_id: call_id.into(),
                name: name.into(),
                arguments,
                output: None,
                status: ToolStatus::Pending,
                run_id: Some(run),
            },
            now_ms,
        )
    }

    /// Marks a recorded tool call as running.
    pub fn start_tool(&mut self, id: MessageId) {
        if let Some(Message {
            content: MessageContent::Tool { status, .. },
            ..
        }) = self.messages.iter_mut().find(|m| m.id == id)
        {
            *status = ToolStatus::Running;
        }
    }

    /// Ends every tool call that has not finished (a stopped run).
    pub fn cancel_open_tools(&mut self, now_ms: u64) {
        let open: Vec<MessageId> = self
            .messages
            .iter()
            .filter(|m| {
                matches!(
                    &m.content,
                    MessageContent::Tool { status, .. }
                        if matches!(status, ToolStatus::Pending | ToolStatus::Running)
                )
            })
            .map(|m| m.id)
            .collect();
        for id in open {
            self.finish_tool(id, "cancelled", ToolStatus::Cancelled, now_ms);
        }
    }

    pub fn finish_tool(
        &mut self,
        id: MessageId,
        result: &str,
        new_status: ToolStatus,
        now_ms: u64,
    ) {
        if let Some(Message {
            content: MessageContent::Tool { output, status, .. },
            ..
        }) = self.messages.iter_mut().find(|m| m.id == id)
        {
            *output = Some(result.to_string());
            *status = new_status;
        }
        self.updated_at_ms = now_ms;
    }

    /// Changes an agent's workspace (the caller makes sure no run is active).
    pub fn set_workspace(&mut self, path: PathBuf) -> Result<(), SessionError> {
        if self.mode != SessionMode::Agent {
            return Err(SessionError::InvalidWorkspace(
                "only an agent has a workspace".into(),
            ));
        }
        check_workspace(&path)?;
        self.workspace = Some(path);
        Ok(())
    }

    /// Turns one tool on or off for this session.
    pub fn set_tool_enabled(&mut self, name: &str, enabled: bool) {
        if self.effective_tools.tools.get(name) != Some(&enabled) {
            self.effective_tools.tools.insert(name.to_string(), enabled);
            self.effective_tools.revision += 1;
        }
    }

    /// Sets a capability's permission (`files.read` and so on) for this session.
    pub fn set_capability(&mut self, key: &str, permission: Permission) {
        if self.effective_tools.capabilities.get(key) != Some(&permission) {
            self.effective_tools
                .capabilities
                .insert(key.to_string(), permission);
            self.effective_tools.revision += 1;
        }
    }

    pub fn append_assistant(&mut self, id: MessageId, content: &str, thinking: &str) {
        let Some(m) = self.messages.iter_mut().find(|m| m.id == id) else {
            return;
        };
        if let MessageContent::Assistant {
            text, thinking: t, ..
        } = &mut m.content
        {
            text.push_str(content);
            if !thinking.is_empty() {
                t.get_or_insert_with(String::new).push_str(thinking);
            }
        }
    }

    /// Records what Ollama reported for a reply.
    pub fn set_reply_stats(&mut self, id: MessageId, new: ReplyStats) {
        if let Some(Message {
            content: MessageContent::Assistant { stats, .. },
            ..
        }) = self.messages.iter_mut().find(|m| m.id == id)
        {
            *stats = Some(new);
        }
    }

    pub fn finish_assistant(
        &mut self,
        id: MessageId,
        new_status: AssistantStatus,
        new_error: Option<String>,
        now_ms: u64,
    ) {
        if let Some(Message {
            content: MessageContent::Assistant { status, error, .. },
            ..
        }) = self.messages.iter_mut().find(|m| m.id == id)
        {
            *status = new_status;
            *error = new_error;
        }
        self.run_state = RunState::Idle;
        self.updated_at_ms = now_ms;
    }
}

/// The stored form of one session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionDocument {
    pub schema_version: u32,
    pub session: Session,
}

impl SessionDocument {
    pub fn new(session: Session) -> SessionDocument {
        SessionDocument {
            schema_version: SESSION_SCHEMA_VERSION,
            session,
        }
    }

    /// The session, if this document's version is one this build understands
    /// and its stored workspace still satisfies the creation rules.
    pub fn into_current(self) -> Result<Session, SessionError> {
        if self.schema_version == 0 || self.schema_version > SESSION_SCHEMA_VERSION {
            return Err(SessionError::UnsupportedSchema(self.schema_version));
        }
        if let Some(workspace) = &self.session.workspace {
            check_workspace(workspace)?;
        }
        Ok(self.session)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const T0: u64 = 1_000;

    fn abs(p: &str) -> PathBuf {
        std::env::temp_dir().join(p)
    }

    fn chat() -> Session {
        Session::new_chat("llama3:8b", SessionLifetime::Persisted, T0)
    }

    fn msg(content: MessageContent) -> Message {
        Message {
            id: MessageId::new(),
            created_at_ms: T0,
            content,
        }
    }

    fn user(text: &str) -> Message {
        msg(MessageContent::User {
            text: text.into(),
            delivery: UserDelivery::Normal,
        })
    }

    fn assistant(text: &str, status: AssistantStatus) -> Message {
        msg(MessageContent::Assistant {
            text: text.into(),
            status,
            run_id: None,
            turn_id: None,
            error: None,
            thinking: None,
            stats: None,
        })
    }

    fn tool(status: ToolStatus) -> Message {
        msg(MessageContent::Tool {
            call_id: "call-1".into(),
            name: "read_file".into(),
            arguments: serde_json::json!({"path": "src/main.rs", "offset": 1}),
            output: None,
            status,
            run_id: None,
        })
    }

    #[test]
    fn ids_round_trip_through_text_and_json() {
        let s = SessionId::new();
        assert_eq!(s.to_string().parse::<SessionId>().unwrap(), s);
        assert!("nope".parse::<MessageId>().is_err());
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(json, format!("\"{s}\""));
        assert_eq!(serde_json::from_str::<SessionId>(&json).unwrap(), s);
        assert_ne!(RunId::new(), RunId::new());
        let mut set = std::collections::HashSet::new();
        assert!(set.insert(TurnId::new()));
        assert!(SessionId::new() != SessionId::new());
    }

    #[test]
    fn chat_defaults_cover_every_field() {
        let s = chat();
        assert_eq!(s.mode, SessionMode::Chat);
        assert_eq!(s.lifetime, SessionLifetime::Persisted);
        assert_eq!(s.title, "Untitled");
        assert_eq!(
            s.title_metadata,
            TitleMetadata {
                auto_title: true,
                last_titled_at_ms: None,
                undo_title: None
            }
        );
        assert!(!s.pinned);
        assert_eq!(s.model, "llama3:8b");
        assert_eq!(s.model_options, ModelOptions::default());
        assert_eq!(s.system_prompt, SystemPromptKind::Chat);
        assert_eq!(s.workspace, None);
        assert_eq!(s.effective_tools, EffectiveToolConfig::default());
        assert_eq!(s.step_budget, 20);
        assert_eq!(s.run_state, RunState::Idle);
        assert_eq!(s.parent, None);
        assert_eq!((s.created_at_ms, s.updated_at_ms), (T0, T0));
        assert!(s.messages.is_empty());
        assert!(!s.is_branch());
        assert!(s.is_idle());
    }

    #[test]
    fn agent_needs_an_absolute_workspace_without_parent_components() {
        let ws = abs("work");
        let s = Session::new_agent("qwen3", SessionLifetime::Persisted, ws.clone(), T0).unwrap();
        assert_eq!(s.mode, SessionMode::Agent);
        assert_eq!(s.system_prompt, SystemPromptKind::Agent);
        assert_eq!(s.workspace, Some(ws));
        assert_eq!(s.step_budget, 20);
        for bad in [
            PathBuf::from("relative/dir"),
            abs("work").join("..").join("escape"),
            PathBuf::new(),
        ] {
            let e = Session::new_agent("qwen3", SessionLifetime::Persisted, bad.clone(), T0)
                .unwrap_err();
            assert!(matches!(e, SessionError::InvalidWorkspace(_)), "{bad:?}");
        }
    }

    #[test]
    fn lifetime_is_independent_of_mode() {
        for lifetime in [SessionLifetime::Persisted, SessionLifetime::Incognito] {
            assert_eq!(Session::new_chat("m", lifetime, T0).lifetime, lifetime);
            let a = Session::new_agent("m", lifetime, abs("w"), T0).unwrap();
            assert_eq!(a.lifetime, lifetime);
        }
    }

    fn full_session() -> Session {
        let mut s =
            Session::new_agent("qwen3:14b", SessionLifetime::Persisted, abs("ws"), T0).unwrap();
        s.title = "Größe 日本語 🦜".into();
        s.pinned = true;
        s.model_options = ModelOptions {
            num_ctx: Some(16_384),
            temperature: Some(0.7),
            seed: Some(-42),
            think: Some(false),
        };
        s.title_metadata = TitleMetadata {
            auto_title: false,
            last_titled_at_ms: Some(5_000),
            undo_title: Some("Old".into()),
        };
        s.effective_tools = EffectiveToolConfig {
            revision: 3,
            capabilities: [("files.read".to_string(), Permission::Auto)].into(),
            tools: [
                ("read_file".to_string(), true),
                ("find_files".to_string(), false),
            ]
            .into(),
        };
        s.parent = Some(BranchOrigin {
            session_id: SessionId::new(),
            message_id: MessageId::new(),
        });
        let (run_id, turn_id) = (RunId::new(), TurnId::new());
        s.run_state = RunState::Running { run_id, turn_id };
        s.messages = vec![
            user("héllo 世界"),
            msg(MessageContent::Assistant {
                text: "ok 🦜".into(),
                status: AssistantStatus::Complete,
                run_id: Some(run_id),
                turn_id: Some(turn_id),
                error: None,
                thinking: Some("hm".into()),
                stats: Some(ReplyStats {
                    prompt_tokens: Some(3),
                    reply_tokens: Some(4),
                    total_ms: Some(5),
                    load_ms: None,
                    eval_ms: Some(2),
                }),
            }),
            msg(MessageContent::Tool {
                call_id: "c1".into(),
                name: "search_text".into(),
                arguments: serde_json::json!({"pattern": "fn \\w+", "nested": [1, 2.5, null]}),
                output: Some("src/a.rs:1: fn main".into()),
                status: ToolStatus::Complete,
                run_id: Some(run_id),
            }),
            msg(MessageContent::User {
                text: "also this".into(),
                delivery: UserDelivery::Steering,
            }),
        ];
        s
    }

    #[test]
    fn a_full_session_round_trips_with_its_schema_version() {
        let s = full_session();
        let json = serde_json::to_string(&SessionDocument::new(s.clone())).unwrap();
        let doc: SessionDocument = serde_json::from_str(&json).unwrap();
        assert_eq!(doc.schema_version, SESSION_SCHEMA_VERSION);
        assert_eq!(doc.into_current().unwrap(), s);
        assert!(json.contains("\"schema_version\":1"), "{json}");
    }

    #[test]
    fn unknown_schema_versions_are_rejected() {
        for v in [0, SESSION_SCHEMA_VERSION + 1, u32::MAX] {
            let mut doc = SessionDocument::new(chat());
            doc.schema_version = v;
            assert_eq!(
                doc.into_current().unwrap_err(),
                SessionError::UnsupportedSchema(v)
            );
        }
    }

    #[test]
    fn is_idle_is_false_while_anything_is_active() {
        let mut s = chat();
        assert!(s.is_idle());
        let (run_id, turn_id) = (RunId::new(), TurnId::new());
        for state in [
            RunState::WaitingForModel { run_id, turn_id },
            RunState::Running { run_id, turn_id },
            RunState::Stopping { run_id },
        ] {
            s.run_state = state;
            assert!(!s.is_idle(), "{state:?}");
        }
        s.run_state = RunState::Idle;
        s.messages = vec![user("a"), assistant("b", AssistantStatus::Streaming)];
        assert!(!s.is_idle());
        for status in [ToolStatus::Pending, ToolStatus::Running] {
            s.messages = vec![tool(status)];
            assert!(!s.is_idle(), "{status:?}");
        }
        s.messages = vec![
            tool(ToolStatus::Complete),
            tool(ToolStatus::Failed),
            tool(ToolStatus::Cancelled),
            assistant("done", AssistantStatus::Complete),
        ];
        assert!(s.is_idle());
    }

    #[test]
    fn loading_an_active_session_normalizes_it_to_idle() {
        let mut s = chat();
        s.run_state = RunState::Running {
            run_id: RunId::new(),
            turn_id: TurnId::new(),
        };
        s.messages = vec![
            user("q"),
            assistant("done", AssistantStatus::Complete),
            assistant("par", AssistantStatus::Streaming),
            tool(ToolStatus::Running),
            tool(ToolStatus::Pending),
            tool(ToolStatus::Complete),
        ];
        assert!(s.normalize_after_crash());
        assert_eq!(s.run_state, RunState::Idle);
        assert!(s.is_idle());
        let statuses: Vec<_> = s
            .messages
            .iter()
            .map(|m| match &m.content {
                MessageContent::User { .. } => "user".to_string(),
                MessageContent::Assistant { status, .. } => format!("{status:?}"),
                MessageContent::Tool { status, .. } => format!("{status:?}"),
            })
            .collect();
        assert_eq!(
            statuses,
            [
                "user",
                "Complete",
                "Interrupted",
                "Cancelled",
                "Cancelled",
                "Complete"
            ]
        );
        // Text of the partial reply is kept, and a second pass changes nothing.
        assert!(matches!(
            &s.messages[2].content,
            MessageContent::Assistant { text, .. } if text == "par"
        ));
        assert!(!s.normalize_after_crash());
    }

    #[test]
    fn only_complete_assistant_messages_are_branch_boundaries() {
        let mut s = chat();
        s.messages = vec![
            user("q1"),
            assistant("a1", AssistantStatus::Complete),
            tool(ToolStatus::Complete),
            assistant("a2", AssistantStatus::Streaming),
            assistant("a3", AssistantStatus::Interrupted),
            assistant("a4", AssistantStatus::Failed),
            assistant("a5", AssistantStatus::Cancelled),
            user("q2"),
        ];
        let id = |i: usize| s.messages[i].id;
        for bad in [0, 2, 3, 4, 5, 6, 7] {
            assert_eq!(
                s.completed_prefix_through(id(bad)).unwrap_err(),
                SessionError::NotABranchBoundary,
                "index {bad}"
            );
        }
        assert_eq!(
            s.completed_prefix_through(MessageId::new()).unwrap_err(),
            SessionError::UnknownMessage
        );
        let prefix = s.completed_prefix_through(id(1)).unwrap();
        assert_eq!(prefix, s.messages[..2]);
        assert_eq!(prefix[1].id, id(1));
    }

    #[test]
    fn prefix_keeps_earlier_tool_records_and_excludes_later_messages() {
        let mut s = chat();
        s.messages = vec![
            user("q"),
            tool(ToolStatus::Complete),
            assistant("a1", AssistantStatus::Complete),
            user("q2"),
            assistant("a2", AssistantStatus::Complete),
        ];
        let prefix = s.completed_prefix_through(s.messages[2].id).unwrap();
        assert_eq!(prefix.len(), 3);
        assert_eq!(prefix, s.messages[..3]);
    }

    #[test]
    fn rename_trims_records_undo_and_disables_auto_title() {
        let mut s = chat();
        s.rename("  My plan  ").unwrap();
        assert_eq!(s.title, "My plan");
        assert_eq!(s.title_metadata.undo_title.as_deref(), Some("Untitled"));
        assert!(!s.title_metadata.auto_title);
        assert_eq!(s.updated_at_ms, T0);
        s.rename("Second").unwrap();
        assert_eq!(s.title_metadata.undo_title.as_deref(), Some("My plan"));
    }

    #[test]
    fn rename_rejects_invalid_titles_without_changing_anything() {
        let mut s = chat();
        s.rename("Keep").unwrap();
        let before = s.clone();
        for bad in ["", "   \t\n", &"x".repeat(81), &"日".repeat(81)] {
            assert_eq!(
                s.rename(bad).unwrap_err(),
                SessionError::InvalidTitle,
                "{bad:?}"
            );
            assert_eq!(s, before);
        }
        // 80 Unicode scalar values (not bytes) are allowed.
        s.rename(&"日".repeat(80)).unwrap();
        assert_eq!(s.title.chars().count(), 80);
    }

    #[test]
    fn stored_sessions_are_revalidated_when_loaded() {
        for bad in [PathBuf::from("relative"), abs("w").join("..").join("x")] {
            let mut s = Session::new_agent("m", SessionLifetime::Persisted, abs("w"), T0).unwrap();
            s.workspace = Some(bad.clone());
            let doc = SessionDocument::new(s);
            assert!(
                matches!(doc.into_current(), Err(SessionError::InvalidWorkspace(_))),
                "{bad:?}"
            );
        }
        let ok = SessionDocument::new(chat());
        assert!(ok.into_current().is_ok());
    }

    #[test]
    fn renaming_to_the_same_title_keeps_the_original_undo() {
        let mut s = chat();
        s.title = "Auto title".into();
        s.rename("My title").unwrap();
        s.rename(" My title ").unwrap();
        assert_eq!(s.title_metadata.undo_title.as_deref(), Some("Auto title"));
        assert_eq!(s.title, "My title");
    }

    #[test]
    fn permission_names_are_kebab_case() {
        assert_eq!(
            serde_json::to_string(&Permission::Auto).unwrap(),
            "\"auto\""
        );
        assert_eq!(serde_json::to_string(&Permission::Ask).unwrap(), "\"ask\"");
        assert_eq!(serde_json::to_string(&Permission::Off).unwrap(), "\"off\"");
    }

    #[test]
    fn set_pinned_does_not_change_updated_at() {
        let mut s = chat();
        s.set_pinned(true);
        assert!(s.pinned);
        assert_eq!(s.updated_at_ms, T0);
    }

    #[test]
    fn reply_stats_give_a_speed_only_with_a_generation_time() {
        let s = ReplyStats {
            reply_tokens: Some(40),
            eval_ms: Some(2_000),
            ..ReplyStats::default()
        };
        assert_eq!(s.tokens_per_second(), Some(20.0));
        assert_eq!(ReplyStats::default().tokens_per_second(), None);
        let zero = ReplyStats {
            reply_tokens: Some(1),
            eval_ms: Some(0),
            ..ReplyStats::default()
        };
        assert_eq!(zero.tokens_per_second(), None);
    }

    #[test]
    fn push_user_appends_normal_user_message_and_bumps_updated_at() {
        let mut s = chat();
        let id = s.push_user("hi", T0 + 5);
        assert_eq!(s.updated_at_ms, T0 + 5);
        assert_eq!(s.messages[0].id, id);
        assert!(matches!(
            &s.messages[0].content,
            MessageContent::User { text, delivery: UserDelivery::Normal } if text == "hi"
        ));
    }

    #[test]
    fn begin_append_finish_builds_one_complete_assistant_message() {
        let mut s = chat();
        let (run, turn) = (RunId::new(), TurnId::new());
        let id = s.begin_assistant(run, turn, T0 + 1);
        assert_eq!(
            s.run_state,
            RunState::Running {
                run_id: run,
                turn_id: turn
            }
        );
        s.append_assistant(id, "Hel", "t1");
        s.append_assistant(id, "lo", "t2");
        s.finish_assistant(id, AssistantStatus::Complete, None, T0 + 9);
        assert_eq!(s.run_state, RunState::Idle);
        assert_eq!(s.updated_at_ms, T0 + 9);
        assert!(matches!(
            &s.messages[0].content,
            MessageContent::Assistant { text, thinking: Some(t), status: AssistantStatus::Complete, .. }
                if text == "Hello" && t == "t1t2"
        ));
    }

    #[test]
    fn old_assistant_json_without_thinking_still_loads() {
        let json = r#"{"role":"assistant","text":"a","status":"complete","run_id":null,"turn_id":null,"error":null}"#;
        let c: MessageContent = serde_json::from_str(json).unwrap();
        assert!(matches!(
            c,
            MessageContent::Assistant { thinking: None, .. }
        ));
        assert!(!serde_json::to_string(&c).unwrap().contains("thinking"));
    }

    #[test]
    fn steering_messages_are_user_messages_marked_as_steering() {
        let mut s = chat();
        let id = s.push_steering("use the other folder", T0 + 3);
        assert_eq!(s.updated_at_ms, T0 + 3);
        assert!(matches!(
            &s.messages.iter().find(|m| m.id == id).unwrap().content,
            MessageContent::User { text, delivery: UserDelivery::Steering } if text == "use the other folder"
        ));
    }

    #[test]
    fn a_tool_call_is_recorded_pending_then_finished_with_its_output() {
        let mut s = chat();
        let run = RunId::new();
        let id = s.push_tool_call(
            run,
            "c1",
            "read_file",
            serde_json::json!({"path": "a"}),
            T0 + 1,
        );
        assert!(!s.is_idle(), "a pending tool call keeps the session busy");
        s.finish_tool(id, "1| hi", ToolStatus::Complete, T0 + 2);
        assert!(s.is_idle());
        let MessageContent::Tool {
            call_id,
            name,
            arguments,
            output,
            status,
            run_id,
        } = &s.messages[0].content
        else {
            panic!("not a tool message")
        };
        assert_eq!((call_id.as_str(), name.as_str()), ("c1", "read_file"));
        assert_eq!(arguments["path"], "a");
        assert_eq!(output.as_deref(), Some("1| hi"));
        assert_eq!((*status, *run_id), (ToolStatus::Complete, Some(run)));
        assert_eq!(s.updated_at_ms, T0 + 2);
    }

    #[test]
    fn only_an_agent_can_change_its_workspace_and_only_to_a_valid_path() {
        let mut agent = Session::new_agent("m", SessionLifetime::Persisted, abs("ws"), T0).unwrap();
        agent.set_workspace(abs("other")).unwrap();
        assert_eq!(agent.workspace, Some(abs("other")));
        assert!(agent.set_workspace(PathBuf::from("relative")).is_err());
        assert!(agent.set_workspace(abs("a").join("..").join("b")).is_err());
        assert_eq!(
            agent.workspace,
            Some(abs("other")),
            "a refused change changes nothing"
        );
        let mut c = chat();
        assert!(
            c.set_workspace(abs("ws")).is_err(),
            "a chat has no workspace"
        );
    }

    #[test]
    fn tool_settings_bump_the_revision_only_when_something_changes() {
        let mut s = chat();
        let r0 = s.effective_tools.revision;
        s.set_tool_enabled("search_text", false);
        assert_eq!(s.effective_tools.tools.get("search_text"), Some(&false));
        assert_eq!(s.effective_tools.revision, r0 + 1);
        s.set_tool_enabled("search_text", false);
        assert_eq!(s.effective_tools.revision, r0 + 1);
        s.set_capability("files.read", Permission::Off);
        assert_eq!(
            s.effective_tools.capabilities.get("files.read"),
            Some(&Permission::Off)
        );
        assert_eq!(s.effective_tools.revision, r0 + 2);
    }

    #[test]
    fn open_tool_calls_can_be_started_and_cancelled_together() {
        let mut s = chat();
        let run = RunId::new();
        let a = s.push_tool_call(run, "c1", "list_dir", serde_json::json!({}), T0 + 1);
        let b = s.push_tool_call(run, "c2", "read_file", serde_json::json!({}), T0 + 1);
        let done = s.push_tool_call(run, "c3", "find_files", serde_json::json!({}), T0 + 1);
        s.finish_tool(done, "ok", ToolStatus::Complete, T0 + 1);
        s.start_tool(a);
        let status =
            |s: &Session, id| match &s.messages.iter().find(|m| m.id == id).unwrap().content {
                MessageContent::Tool { status, .. } => *status,
                _ => panic!(),
            };
        assert_eq!(status(&s, a), ToolStatus::Running);
        s.cancel_open_tools(T0 + 5);
        assert_eq!(status(&s, a), ToolStatus::Cancelled);
        assert_eq!(status(&s, b), ToolStatus::Cancelled);
        assert_eq!(
            status(&s, done),
            ToolStatus::Complete,
            "finished calls are left alone"
        );
        assert!(s.is_idle());
    }
}
