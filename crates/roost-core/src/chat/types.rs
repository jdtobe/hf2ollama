//! The vocabulary between the GUI and the chat manager: intent commands in,
//! ordered events out.

use crate::session::{Message, SessionId, SessionLifetime, TurnId};
use crate::store::IndexRow;
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnOutcome {
    Completed,
    Failed,
    Cancelled,
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
