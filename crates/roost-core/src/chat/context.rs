//! What goes into a chat request: the window Ollama is told to load, and the
//! deterministic trimming of old message groups to fit it.

use crate::session::{AssistantStatus, Message, MessageContent};
use serde::Serialize;

/// Largest window Roost asks Ollama to load; bounds KV-cache memory.
pub const WINDOW_CAP: u32 = 16_384;
/// Used when neither the session nor `/api/show` gives a context length.
pub const FALLBACK_WINDOW: u32 = 8_192;
pub const OMITTED_MARKER: &str = "Earlier messages omitted for context";

/// One message as sent to `/api/chat`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WireMessage {
    pub role: String,
    pub content: String,
    /// Set on an assistant message that asked for tools.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<WireToolCall>>,
    /// Set on a `tool` message: which tool produced the content.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
}

impl WireMessage {
    pub fn text(role: &str, content: &str) -> WireMessage {
        WireMessage {
            role: role.into(),
            content: content.into(),
            tool_calls: None,
            tool_name: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WireToolCall {
    pub function: WireFunction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WireFunction {
    pub name: String,
    pub arguments: serde_json::Value,
}

impl WireToolCall {
    pub fn new(name: &str, arguments: serde_json::Value) -> WireToolCall {
        WireToolCall {
            function: WireFunction {
                name: name.into(),
                arguments,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trimmed {
    pub messages: Vec<WireMessage>,
    /// Messages left out because they did not fit (not failed exchanges).
    pub omitted: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ContextError {
    #[error("this message is too large for the model's context window ({needed} tokens needed, {budget} available)")]
    TooLarge { needed: usize, budget: usize },
}

/// The context window to request: the session override, else the model's
/// reported maximum capped at `WINDOW_CAP`, else `FALLBACK_WINDOW`.
pub fn window(session_num_ctx: Option<u32>, show_context: Option<u64>) -> u32 {
    session_num_ctx.unwrap_or(match show_context {
        Some(c) if c > 0 => c.min(u64::from(WINDOW_CAP)) as u32,
        _ => FALLBACK_WINDOW,
    })
}

/// `ceil(UTF-8 bytes / 3) + 8` per message.
fn tokens(text: &str) -> usize {
    text.len().div_ceil(3) + 8
}

/// A user message and the complete reply that answered it, if any.
struct Exchange<'a> {
    user: &'a str,
    reply: Option<&'a str>,
}

fn exchanges(transcript: &[Message]) -> Vec<Exchange<'_>> {
    let mut out: Vec<Exchange> = Vec::new();
    for m in transcript {
        match &m.content {
            MessageContent::User { text, .. } => out.push(Exchange {
                user: text,
                reply: None,
            }),
            MessageContent::Assistant {
                text,
                status: AssistantStatus::Complete,
                ..
            } => {
                if let Some(last) = out.last_mut() {
                    last.reply = Some(text);
                }
            }
            _ => {}
        }
    }
    out
}

fn wire(role: &str, content: &str) -> WireMessage {
    WireMessage::text(role, content)
}

/// Builds the messages for one request. Exchanges whose reply did not
/// complete are dropped whole when a later user message exists, so a request
/// never has two user turns in a row. Old complete exchanges are dropped
/// oldest-first when they do not fit in 70% of the window.
pub fn build_request(
    system: &str,
    transcript: &[Message],
    window: u32,
) -> Result<Trimmed, ContextError> {
    let mut all = exchanges(transcript);
    let newest = all.pop();
    let fixed = tokens(system) + newest.as_ref().map_or(0, |e| tokens(e.user));
    let budget = (u64::from(window) * 70 / 100) as usize;
    if fixed > budget {
        return Err(ContextError::TooLarge {
            needed: fixed,
            budget,
        });
    }
    let history: Vec<&Exchange> = all.iter().filter(|e| e.reply.is_some()).collect();
    let cost = |e: &Exchange| tokens(e.user) + e.reply.map_or(0, tokens);
    let take = |room: usize| -> usize {
        let mut used = 0;
        let mut kept = 0;
        for e in history.iter().rev() {
            used += cost(e);
            if used > room {
                break;
            }
            kept += 1;
        }
        kept
    };
    let room = budget - fixed;
    let mut kept = take(room);
    let mut marker = false;
    if kept < history.len() {
        marker = true;
        kept = take(room.saturating_sub(tokens(OMITTED_MARKER)));
    }
    let mut messages = vec![wire("system", system)];
    if marker {
        messages.push(wire("system", OMITTED_MARKER));
    }
    for e in &history[history.len() - kept..] {
        messages.push(wire("user", e.user));
        if let Some(r) = e.reply {
            messages.push(wire("assistant", r));
        }
    }
    if let Some(e) = newest {
        messages.push(wire("user", e.user));
    }
    Ok(Trimmed {
        messages,
        omitted: (history.len() - kept) * 2,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{AssistantStatus as St, RunId, Session, SessionLifetime, TurnId};

    fn session() -> Session {
        Session::new_chat("m", SessionLifetime::Persisted, 0)
    }

    fn exchange(s: &mut Session, user: &str, reply: Option<(&str, St)>) {
        s.push_user(user, 1);
        if let Some((text, status)) = reply {
            let id = s.begin_assistant(RunId::new(), TurnId::new(), 1);
            s.append_assistant(id, text, "secret thoughts");
            s.finish_assistant(id, status, None, 1);
        }
    }

    fn roles(t: &Trimmed) -> Vec<&str> {
        t.messages.iter().map(|m| m.role.as_str()).collect()
    }

    #[test]
    fn window_prefers_session_override() {
        assert_eq!(window(Some(4096), Some(131_072)), 4096);
    }

    #[test]
    fn window_caps_show_context_at_16384() {
        assert_eq!(window(None, Some(131_072)), WINDOW_CAP);
        assert_eq!(window(None, Some(4096)), 4096);
    }

    #[test]
    fn window_falls_back_to_8192() {
        assert_eq!(window(None, None), FALLBACK_WINDOW);
        assert_eq!(window(None, Some(0)), FALLBACK_WINDOW);
    }

    // Each 3-byte text costs ceil(3/3) + 8 = 9 tokens. system + newest user +
    // one old exchange (user + reply) = 9 + 9 + 18 = 36 tokens.
    fn small() -> (Session, &'static str) {
        let mut s = session();
        exchange(&mut s, "aaa", Some(("bbb", St::Complete)));
        exchange(&mut s, "uuu", None);
        (s, "sss")
    }

    #[test]
    fn exact_fit_keeps_everything() {
        let (s, sys) = small();
        // window 52 -> budget 52 * 70 / 100 = 36
        let t = build_request(sys, &s.messages, 52).unwrap();
        assert_eq!(roles(&t), ["system", "user", "assistant", "user"]);
        assert_eq!(t.omitted, 0);
    }

    #[test]
    fn one_token_over_drops_the_oldest_group_and_adds_marker() {
        let (s, sys) = small();
        // window 51 -> budget 35: the old exchange no longer fits
        let t = build_request(sys, &s.messages, 51).unwrap();
        assert_eq!(roles(&t), ["system", "system", "user"]);
        assert_eq!(t.messages[1].content, OMITTED_MARKER);
        assert_eq!(t.omitted, 2);
    }

    #[test]
    fn never_starts_with_an_assistant_message() {
        let mut s = session();
        exchange(&mut s, "one", Some(("1", St::Complete)));
        exchange(&mut s, "two", Some(("2", St::Complete)));
        exchange(&mut s, "three", None);
        for w in 30..200 {
            if let Ok(t) = build_request("s", &s.messages, w) {
                let first_non_system = t.messages.iter().find(|m| m.role != "system").unwrap();
                assert_eq!(first_non_system.role, "user", "window {w}");
            }
        }
    }

    #[test]
    fn failed_exchange_before_a_new_user_message_is_dropped_whole_and_not_counted() {
        let mut s = session();
        exchange(&mut s, "first", Some(("partial", St::Failed)));
        exchange(&mut s, "second", None);
        let t = build_request("sys", &s.messages, 8192).unwrap();
        assert_eq!(roles(&t), ["system", "user"]);
        assert_eq!(t.messages[1].content, "second");
        assert_eq!(t.omitted, 0);
    }

    #[test]
    fn newest_user_message_is_kept_when_its_reply_failed() {
        let mut s = session();
        exchange(&mut s, "ask", Some(("half", St::Cancelled)));
        let t = build_request("sys", &s.messages, 8192).unwrap();
        assert_eq!(roles(&t), ["system", "user"]);
        assert_eq!(t.messages[1].content, "ask");
    }

    #[test]
    fn a_retried_exchange_uses_its_complete_reply() {
        let mut s = session();
        s.push_user("q", 1);
        for (text, st) in [("bad", St::Failed), ("good", St::Complete)] {
            let id = s.begin_assistant(RunId::new(), TurnId::new(), 1);
            s.append_assistant(id, text, "");
            s.finish_assistant(id, st, None, 1);
        }
        exchange(&mut s, "next", None);
        let t = build_request("sys", &s.messages, 8192).unwrap();
        assert_eq!(roles(&t), ["system", "user", "assistant", "user"]);
        assert_eq!(t.messages[2].content, "good");
    }

    #[test]
    fn thinking_is_never_sent() {
        let mut s = session();
        exchange(&mut s, "q", Some(("a", St::Complete)));
        exchange(&mut s, "q2", None);
        let t = build_request("sys", &s.messages, 8192).unwrap();
        assert!(t.messages.iter().all(|m| !m.content.contains("secret")));
    }

    #[test]
    fn system_plus_newest_too_large_is_an_error() {
        let mut s = session();
        exchange(&mut s, &"x".repeat(900), None);
        let err = build_request("sys", &s.messages, 100).unwrap_err();
        assert!(matches!(err, ContextError::TooLarge { budget: 70, .. }));
        assert!(err.to_string().contains("too large"));
    }

    #[test]
    fn the_transcript_itself_is_never_modified() {
        let (s, sys) = small();
        let before = s.messages.clone();
        let _ = build_request(sys, &s.messages, 51).unwrap();
        assert_eq!(s.messages, before);
    }
}
