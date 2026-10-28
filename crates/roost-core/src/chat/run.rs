//! The task that talks to Ollama for one turn. It owns no session state: it
//! only reports what the stream did, and the manager decides what that means.

use super::manager::ModelInfo;
use crate::agent::ToolError;
use crate::error::Error;
use crate::ollama::{ChatRequestBody, OllamaClient, StreamEvent};
use crate::session::{RunId, SessionId, TurnId};
use tokio::sync::{mpsc::UnboundedSender, watch};

pub(super) enum RunMsg {
    /// The answer to a `show` request made while preparing a send.
    Shown {
        session: SessionId,
        token: u64,
        info: Result<ModelInfo, Error>,
    },
    /// The answer to `CheckModel`.
    Capabilities {
        model: String,
        info: Result<ModelInfo, Error>,
    },
    /// A tool call finished on the blocking pool.
    ToolDone {
        session: SessionId,
        run: RunId,
        call_id: String,
        result: Result<String, ToolError>,
    },
    Event {
        session: SessionId,
        turn: TurnId,
        event: StreamEvent,
    },
    Failed {
        session: SessionId,
        turn: TurnId,
        error: Error,
    },
}

/// Streams one reply. Returns silently when told to stop; the manager has
/// already recorded the outcome by then.
pub(super) async fn drive(
    client: OllamaClient,
    body: ChatRequestBody,
    session: SessionId,
    turn: TurnId,
    tx: UnboundedSender<RunMsg>,
    mut stop: watch::Receiver<bool>,
) {
    let opened = tokio::select! {
        r = client.chat_stream(&body) => r,
        _ = stop.changed() => return,
    };
    let mut stream = match opened {
        Ok(s) => s,
        Err(error) => {
            let _ = tx.send(RunMsg::Failed {
                session,
                turn,
                error,
            });
            return;
        }
    };
    loop {
        let next = tokio::select! {
            r = stream.next() => r,
            _ = stop.changed() => return,
        };
        match next {
            Ok(Some(event)) => {
                let done = matches!(event, StreamEvent::Done { .. });
                let _ = tx.send(RunMsg::Event {
                    session,
                    turn,
                    event,
                });
                if done {
                    return;
                }
            }
            Ok(None) => return,
            Err(error) => {
                let _ = tx.send(RunMsg::Failed {
                    session,
                    turn,
                    error,
                });
                return;
            }
        }
    }
}
