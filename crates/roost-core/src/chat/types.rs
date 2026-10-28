//! The vocabulary between the GUI and the chat manager: intent commands in,
//! ordered events out.

use crate::session::{Message, MessageId, Permission, SessionId, SessionLifetime, TurnId};
use crate::store::IndexRow;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub enum ChatCommand {
    Create {
        lifetime: SessionLifetime,
        model: String,
    },
    Open(SessionId),
    /// View-level for saved sessions; destroys an incognito one.
    Close(SessionId),
    ClearIncognito,
    /// An agent: a chat with a workspace folder and read-only tools.
    CreateAgent {
        lifetime: SessionLifetime,
        model: String,
        workspace: PathBuf,
    },
    /// Ask whether a model advertises tool support (answered by `ModelCapabilities`).
    CheckModel(String),
    /// Change an idle agent's workspace folder.
    SetWorkspace(SessionId, PathBuf),
    /// How many model steps an agent run may use (1 to 100).
    SetStepBudget(SessionId, u32),
    SetToolEnabled(SessionId, String, bool),
    SetCapability(SessionId, String, Permission),
    Rename(SessionId, String),
    SetPinned(SessionId, bool),
    Delete(SessionId),
    SetModel(SessionId, String),
    Send {
        session: SessionId,
        text: String,
    },
    Stop(SessionId),
    Retry(SessionId),
    /// Drop the last complete reply and ask again (chats only).
    Regenerate(SessionId),
    ListSessions,
    /// The installed models changed: forget cached context lengths.
    InstalledChanged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendRejection {
    /// A reply is already being produced.
    Busy,
    /// Readable explanation of why the message cannot fit.
    TooLarge(String),
    UnknownSession,
    EmptyMessage,
    /// Stopped while preparing, before anything was sent.
    Cancelled,
    /// The model does not advertise tool support, so it cannot run an agent.
    NoToolSupport(String),
    /// The agent's workspace folder is missing or cannot be opened.
    NoWorkspace(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnOutcome {
    Completed,
    Failed,
    Cancelled,
    /// The user steered an agent: the partial reply is kept.
    Interrupted,
}

/// How a whole run (one chat reply, or an agent's loop of steps) ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOutcome {
    Completed,
    Failed,
    Cancelled,
    /// An agent used all its steps.
    BudgetExhausted,
}

#[derive(Debug, Clone)]
pub enum ChatEvent {
    /// Every session row, persisted and incognito.
    Rows(Vec<IndexRow>),
    /// A session's transcript, after create or open.
    Opened(Box<crate::session::Session>),
    /// A message added to a session (the user's, then the empty reply).
    Appended {
        session: SessionId,
        message: Box<Message>,
    },
    /// A message left the transcript (a reply replaced by Regenerate).
    Removed {
        session: SessionId,
        message: MessageId,
    },
    TurnStarted {
        session: SessionId,
        turn: TurnId,
    },
    /// Another reply is being generated, so this one may be queued.
    WaitingForModel {
        session: SessionId,
        turn: TurnId,
    },
    Delta {
        session: SessionId,
        turn: TurnId,
        content: String,
        thinking: String,
    },
    ContextTrimmed {
        session: SessionId,
        omitted: usize,
    },
    /// Exactly one per turn. `message` is the readable failure, if any.
    TurnEnded {
        session: SessionId,
        turn: TurnId,
        outcome: TurnOutcome,
        message: Option<String>,
        prompt_eval_count: Option<u64>,
    },
    SendRejected {
        session: SessionId,
        reason: SendRejection,
    },
    /// An agent started a tool call.
    ToolStarted {
        session: SessionId,
        call_id: String,
        name: String,
        arguments: serde_json::Value,
    },
    ToolFinished {
        session: SessionId,
        call_id: String,
        ok: bool,
        output: String,
    },
    /// A message sent to a running agent, waiting for its next safe step.
    SteeringQueued {
        session: SessionId,
        text: String,
    },
    /// Exactly one per run, after its last `TurnEnded`.
    RunEnded {
        session: SessionId,
        outcome: RunOutcome,
        message: Option<String>,
    },
    ModelCapabilities {
        model: String,
        tools: bool,
    },
    /// An idle agent's folder, step budget or tool settings changed.
    AgentSettings {
        session: SessionId,
        workspace: Option<PathBuf>,
        step_budget: u32,
        tools: crate::session::EffectiveToolConfig,
    },
    /// The model is not installed; the user should pick another.
    ModelMissing {
        session: SessionId,
        model: String,
    },
    StoreWarning(String),
    SessionError {
        session: Option<SessionId>,
        message: String,
    },
}

pub type EventSink = Arc<dyn Fn(ChatEvent) + Send + Sync>;
