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

/// One unit of an agent transcript that is kept or dropped as a whole: a user
/// message, or an assistant message together with the tool results that
/// answer its calls.
struct Group {
    wires: Vec<WireMessage>,
    is_user: bool,
    cost: usize,
    messages: usize,
}

fn wire_cost(m: &WireMessage) -> usize {
    let calls: usize = m
        .tool_calls
        .iter()
        .flatten()
        .map(|c| tokens(&c.function.name) + tokens(&c.function.arguments.to_string()))
        .sum();
    tokens(&m.content) + calls
}

fn agent_groups(transcript: &[Message]) -> Vec<Group> {
    let mut groups = Vec::new();
    let mut i = 0;
    while i < transcript.len() {
        match &transcript[i].content {
            MessageContent::User { text, .. } => {
                let w = wire("user", text);
                groups.push(Group {
                    cost: wire_cost(&w),
                    wires: vec![w],
                    is_user: true,
                    messages: 1,
                });
                i += 1;
            }
            MessageContent::Assistant { text, .. } => {
                let mut j = i + 1;
                let mut calls = Vec::new();
                let mut results = Vec::new();
                while let Some(MessageContent::Tool {
                    name,
                    arguments,
                    output,
                    ..
                }) = transcript.get(j).map(|m| &m.content)
                {
                    calls.push(WireToolCall::new(name, arguments.clone()));
                    results.push(WireMessage {
                        tool_name: Some(name.clone()),
                        ..wire("tool", output.as_deref().unwrap_or("cancelled"))
                    });
                    j += 1;
                }
                if !text.is_empty() || !calls.is_empty() {
                    let mut a = wire("assistant", text);
                    if !calls.is_empty() {
                        a.tool_calls = Some(calls);
                    }
                    let mut wires = vec![a];
                    wires.extend(results);
                    groups.push(Group {
                        cost: wires.iter().map(wire_cost).sum(),
                        wires,
                        is_user: false,
                        messages: j - i,
                    });
                }
                i = j;
            }
            // A tool result with no assistant message before it cannot be replayed.
            MessageContent::Tool { .. } => i += 1,
        }
    }
    groups
}

/// Like `build_request`, for agent transcripts: user, assistant and tool
/// messages in order. Tool calls and their results are never separated, the
/// request always starts with a user message, and the newest message and the
/// newest user message are always kept. Interrupted and cancelled replies stay
/// as the text they had.
pub fn build_agent_request(
    system: &str,
    transcript: &[Message],
    window: u32,
) -> Result<Trimmed, ContextError> {
    let groups = agent_groups(transcript);
    let budget = (u64::from(window) * 70 / 100) as usize;
    let newest = groups.len().checked_sub(1);
    let newest_user = groups.iter().rposition(|g| g.is_user);
    let mut required: Vec<usize> = newest.into_iter().chain(newest_user).collect();
    required.sort_unstable();
    required.dedup();
    let fixed = tokens(system) + required.iter().map(|&i| groups[i].cost).sum::<usize>();
    if fixed > budget {
        return Err(ContextError::TooLarge {
            needed: fixed,
            budget,
        });
    }
    let others: Vec<usize> = (0..groups.len())
        .filter(|i| !required.contains(i))
        .collect();
    let pick = |room: usize| -> Vec<usize> {
        let mut used = 0;
        let mut chosen = Vec::new();
        for &i in others.iter().rev() {
            used += groups[i].cost;
            if used > room {
                break;
            }
            chosen.push(i);
        }
        chosen
    };
    let room = budget - fixed;
    let mut chosen = pick(room);
    let mut marker = false;
    if chosen.len() < others.len() {
        marker = true;
        chosen = pick(room.saturating_sub(tokens(OMITTED_MARKER)));
    }
    let mut keep: Vec<usize> = required.into_iter().chain(chosen).collect();
    keep.sort_unstable();
    // Never start in the middle of an exchange.
    while keep.len() > 1 && !groups[keep[0]].is_user {
        keep.remove(0);
    }
    let kept_messages: usize = keep.iter().map(|&i| groups[i].messages).sum();
    let total_messages: usize = groups.iter().map(|g| g.messages).sum();
    let mut messages = vec![wire("system", system)];
    if marker {
        messages.push(wire("system", OMITTED_MARKER));
    }
    for &i in &keep {
        messages.extend(groups[i].wires.iter().cloned());
    }
    Ok(Trimmed {
        messages,
        omitted: total_messages - kept_messages,
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

    // ---- agent requests

    use crate::session::ToolStatus;
    use serde_json::json;

    /// user, assistant asking for tools, their results, a closing reply.
    fn agent_session() -> Session {
        let mut s =
            Session::new_agent("m", SessionLifetime::Persisted, std::env::temp_dir(), 0).unwrap();
        s.push_user("find the bug", 1);
        let run = RunId::new();
        let a = s.begin_assistant(run, TurnId::new(), 2);
        s.append_assistant(a, "Looking.", "private reasoning");
        s.finish_assistant(a, St::Complete, None, 2);
        let t1 = s.push_tool_call(run, "c1", "list_dir", json!({}), 3);
        s.finish_tool(t1, "a.rs", ToolStatus::Complete, 3);
        let t2 = s.push_tool_call(run, "c2", "read_file", json!({"path": "a.rs"}), 3);
        s.finish_tool(t2, "1| fn main() {}", ToolStatus::Complete, 3);
        let b = s.begin_assistant(run, TurnId::new(), 4);
        s.append_assistant(b, "It is in a.rs.", "");
        s.finish_assistant(b, St::Complete, None, 4);
        s
    }

    fn shapes(t: &Trimmed) -> Vec<String> {
        t.messages
            .iter()
            .map(|m| {
                let calls = m.tool_calls.as_ref().map_or(0, Vec::len);
                format!(
                    "{}{}",
                    m.role,
                    if calls > 0 {
                        format!("+{calls}")
                    } else {
                        String::new()
                    }
                )
            })
            .collect()
    }

    #[test]
    fn agent_requests_replay_tool_calls_and_results_in_order() {
        let mut s = agent_session();
        s.push_steering("only look at src", 5);
        let t = build_agent_request("sys", &s.messages, 8192).unwrap();
        assert_eq!(
            shapes(&t),
            [
                "system",
                "user",
                "assistant+2",
                "tool",
                "tool",
                "assistant",
                "user"
            ]
        );
        let calls = t.messages[2].tool_calls.as_ref().unwrap();
        assert_eq!(calls[0].function.name, "list_dir");
        assert_eq!(calls[1].function.arguments["path"], "a.rs");
        assert_eq!(t.messages[3].tool_name.as_deref(), Some("list_dir"));
        assert_eq!(t.messages[3].content, "a.rs");
        assert_eq!(t.messages[4].tool_name.as_deref(), Some("read_file"));
        assert_eq!(t.messages[6].content, "only look at src");
        assert_eq!(t.omitted, 0);
        assert!(t.messages.iter().all(|m| !m.content.contains("private")));
    }

    #[test]
    fn an_interrupted_reply_stays_as_text_and_empty_replies_are_skipped() {
        let mut s =
            Session::new_agent("m", SessionLifetime::Persisted, std::env::temp_dir(), 0).unwrap();
        s.push_user("task", 1);
        let a = s.begin_assistant(RunId::new(), TurnId::new(), 2);
        s.append_assistant(a, "I was about to", "");
        s.finish_assistant(a, St::Interrupted, None, 2);
        let empty = s.begin_assistant(RunId::new(), TurnId::new(), 3);
        s.finish_assistant(empty, St::Failed, Some("boom".into()), 3);
        s.push_steering("do this instead", 4);
        let t = build_agent_request("sys", &s.messages, 8192).unwrap();
        assert_eq!(shapes(&t), ["system", "user", "assistant", "user"]);
        assert_eq!(t.messages[2].content, "I was about to");
    }

    #[test]
    fn a_tool_call_with_no_output_is_reported_as_cancelled() {
        let mut s = agent_session();
        let pending =
            s.push_tool_call(RunId::new(), "c9", "find_files", json!({"pattern": "*"}), 6);
        s.finish_tool(pending, "", ToolStatus::Cancelled, 6);
        if let MessageContent::Tool { output, .. } = &mut s
            .messages
            .iter_mut()
            .find(|m| m.id == pending)
            .unwrap()
            .content
        {
            *output = None;
        }
        s.push_steering("next", 7);
        let t = build_agent_request("sys", &s.messages, 8192).unwrap();
        // The orphan call (no assistant before it in its group) is not replayed.
        assert!(t
            .messages
            .iter()
            .all(|m| m.content != "cancelled" || m.role == "tool"));
    }

    #[test]
    fn trimming_keeps_tool_groups_whole_and_starts_with_a_user_message() {
        let mut s =
            Session::new_agent("m", SessionLifetime::Persisted, std::env::temp_dir(), 0).unwrap();
        for i in 0..12 {
            s.push_user(&format!("task {i} {}", "x".repeat(200)), 1);
            let run = RunId::new();
            let a = s.begin_assistant(run, TurnId::new(), 2);
            s.append_assistant(a, &"y".repeat(200), "");
            s.finish_assistant(a, St::Complete, None, 2);
            let t = s.push_tool_call(run, &format!("c{i}"), "read_file", json!({"path": "a"}), 3);
            s.finish_tool(t, &"z".repeat(300), ToolStatus::Complete, 3);
        }
        for w in 200..4000 {
            let Ok(t) = build_agent_request("s", &s.messages, w) else {
                continue;
            };
            let first = t.messages.iter().find(|m| m.role != "system").unwrap();
            assert_eq!(first.role, "user", "window {w}");
            for (i, m) in t.messages.iter().enumerate() {
                if m.role == "tool" {
                    let before = &t.messages[..i];
                    let owner = before.iter().rev().find(|p| p.role == "assistant");
                    assert!(
                        owner.is_some_and(|o| o.tool_calls.is_some()),
                        "window {w}: orphan tool result"
                    );
                }
            }
            let last_user = s.messages.iter().rev().find_map(|m| match &m.content {
                MessageContent::User { text, .. } => Some(text.clone()),
                _ => None,
            });
            assert!(
                t.messages
                    .iter()
                    .any(|m| Some(&m.content) == last_user.as_ref()),
                "window {w}"
            );
        }
    }

    #[test]
    fn dropped_agent_groups_leave_the_marker_and_a_count() {
        let mut s =
            Session::new_agent("m", SessionLifetime::Persisted, std::env::temp_dir(), 0).unwrap();
        for i in 0..20 {
            s.push_user(&format!("step {i} {}", "x".repeat(400)), 1);
        }
        let t = build_agent_request("sys", &s.messages, 1000).unwrap();
        assert_eq!(t.messages[1].content, OMITTED_MARKER);
        assert!(t.omitted > 0 && t.omitted < 20, "{}", t.omitted);
        assert_eq!(
            t.messages.last().unwrap().content,
            s.messages
                .last()
                .and_then(|m| match &m.content {
                    MessageContent::User { text, .. } => Some(text.clone()),
                    _ => None,
                })
                .unwrap()
        );
    }

    #[test]
    fn an_agent_request_too_large_for_the_window_is_an_error() {
        let mut s =
            Session::new_agent("m", SessionLifetime::Persisted, std::env::temp_dir(), 0).unwrap();
        s.push_user(&"x".repeat(30_000), 1);
        assert!(matches!(
            build_agent_request("sys", &s.messages, 100),
            Err(ContextError::TooLarge { .. })
        ));
    }
}
