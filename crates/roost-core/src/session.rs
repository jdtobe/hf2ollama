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
const UNTITLED: &str = "Untitled";

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
    pub fn rename(&mut self, title: &str, now_ms: u64) -> Result<(), SessionError> {
        let title = title.trim();
        if title.is_empty() || title.chars().count() > MAX_TITLE_CHARS {
            return Err(SessionError::InvalidTitle);
        }
        self.title_metadata.undo_title = Some(std::mem::replace(&mut self.title, title.into()));
        self.title_metadata.auto_title = false;
        self.updated_at_ms = now_ms;
        Ok(())
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

    /// The session, if this document's version is one this build understands.
    pub fn into_current(self) -> Result<Session, SessionError> {
        if self.schema_version == 0 || self.schema_version > SESSION_SCHEMA_VERSION {
            return Err(SessionError::UnsupportedSchema(self.schema_version));
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
        s.rename("  My plan  ", T0 + 50).unwrap();
        assert_eq!(s.title, "My plan");
        assert_eq!(s.title_metadata.undo_title.as_deref(), Some("Untitled"));
        assert!(!s.title_metadata.auto_title);
        assert_eq!(s.updated_at_ms, T0 + 50);
        s.rename("Second", T0 + 60).unwrap();
        assert_eq!(s.title_metadata.undo_title.as_deref(), Some("My plan"));
    }

    #[test]
    fn rename_rejects_invalid_titles_without_changing_anything() {
        let mut s = chat();
        s.rename("Keep", T0 + 1).unwrap();
        let before = s.clone();
        for bad in ["", "   \t\n", &"x".repeat(81), &"日".repeat(81)] {
            assert_eq!(
                s.rename(bad, T0 + 99).unwrap_err(),
                SessionError::InvalidTitle,
                "{bad:?}"
            );
            assert_eq!(s, before);
        }
        // 80 Unicode scalar values (not bytes) are allowed.
        s.rename(&"日".repeat(80), T0 + 2).unwrap();
        assert_eq!(s.title.chars().count(), 80);
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
}
