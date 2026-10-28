//! Streaming `POST /api/chat`.

use super::{map_send, map_send_mid_stream, OllamaClient};
use crate::chat::context::WireMessage;
use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::time::Duration;

pub(super) const CHAT_STALL: Duration = Duration::from_secs(120);
pub(super) const FIRST_EVENT: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, PartialEq)]
pub struct ChatRequestBody {
    pub model: String,
    pub messages: Vec<WireMessage>,
    pub num_ctx: u32,
    pub temperature: Option<f32>,
    pub seed: Option<i64>,
    pub think: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamEvent {
    Content(String),
    Thinking(String),
    Done {
        prompt_eval_count: Option<u64>,
        eval_count: Option<u64>,
        done_reason: Option<String>,
    },
}

#[derive(Serialize)]
struct Options {
    num_ctx: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seed: Option<i64>,
}

#[derive(Serialize)]
struct Wire<'a> {
    model: &'a str,
    messages: &'a [WireMessage],
    stream: bool,
    options: Options,
    #[serde(skip_serializing_if = "Option::is_none")]
    think: Option<bool>,
}

#[derive(Deserialize)]
struct Line {
    #[serde(default)]
    message: Option<LineMessage>,
    #[serde(default)]
    done: bool,
    done_reason: Option<String>,
    prompt_eval_count: Option<u64>,
    eval_count: Option<u64>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct LineMessage {
    #[serde(default)]
    content: String,
    #[serde(default)]
    thinking: String,
}

/// Events from one chat request. `Done` is last, then `next` returns `None`.
#[derive(Debug)]
pub struct ChatStream {
    resp: reqwest::Response,
    buf: Vec<u8>,
    pending: VecDeque<Result<StreamEvent>>,
    stall: Duration,
    first_event: Duration,
    seen_line: bool,
    finished: bool,
}

impl ChatStream {
    pub async fn next(&mut self) -> Result<Option<StreamEvent>> {
        loop {
            if let Some(item) = self.pending.pop_front() {
                if item.is_err() {
                    self.finished = true;
                    self.pending.clear();
                }
                return item.map(Some);
            }
            if self.finished {
                return Ok(None);
            }
            let limit = if self.seen_line {
                self.stall
            } else {
                self.first_event
            };
            let chunk = tokio::time::timeout(limit, self.resp.chunk())
                .await
                .map_err(|_| {
                    if self.seen_line {
                        Error::Network("ollama stalled".into())
                    } else {
                        Error::Network(
                            "Ollama did not answer in time while loading the model".into(),
                        )
                    }
                })?
                .map_err(map_send_mid_stream)?;
            match chunk {
                Some(c) => {
                    self.buf.extend_from_slice(&c);
                    while let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
                        let line: Vec<u8> = self.buf.drain(..=pos).collect();
                        self.parse(&line);
                    }
                }
                None => {
                    if !self.buf.is_empty() {
                        let rest = std::mem::take(&mut self.buf);
                        self.parse(&rest);
                    }
                    let ended_with_done = self
                        .pending
                        .iter()
                        .any(|p| matches!(p, Ok(StreamEvent::Done { .. })));
                    if !ended_with_done && self.pending.iter().all(|p| p.is_ok()) {
                        self.pending
                            .push_back(Err(Error::Network("connection ended early".into())));
                    }
                    self.finished = true;
                }
            }
        }
    }

    fn parse(&mut self, line: &[u8]) {
        let text = String::from_utf8_lossy(line);
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        self.seen_line = true;
        let parsed: Line = match serde_json::from_str(text) {
            Ok(p) => p,
            Err(e) => {
                self.pending.push_back(Err(Error::Parse(e.to_string())));
                return;
            }
        };
        if let Some(msg) = parsed.error {
            self.pending.push_back(Err(Error::Ollama(msg)));
            return;
        }
        if let Some(m) = parsed.message {
            if !m.thinking.is_empty() {
                self.pending
                    .push_back(Ok(StreamEvent::Thinking(m.thinking)));
            }
            if !m.content.is_empty() {
                self.pending.push_back(Ok(StreamEvent::Content(m.content)));
            }
        }
        if parsed.done {
            self.pending.push_back(Ok(StreamEvent::Done {
                prompt_eval_count: parsed.prompt_eval_count,
                eval_count: parsed.eval_count,
                done_reason: parsed.done_reason,
            }));
            self.finished = true;
        }
    }
}

impl OllamaClient {
    /// Starts a streaming chat. A 404 means the model is missing.
    pub async fn chat_stream(&self, body: &ChatRequestBody) -> Result<ChatStream> {
        let wire = Wire {
            model: &body.model,
            messages: &body.messages,
            stream: true,
            options: Options {
                num_ctx: body.num_ctx,
                temperature: body.temperature,
                seed: body.seed,
            },
            think: body.think,
        };
        let url = self.url("/api/chat")?;
        let resp = tokio::time::timeout(self.first_event, self.http.post(url).json(&wire).send())
            .await
            .map_err(|_| {
                Error::Network("Ollama did not answer in time while loading the model".into())
            })?
            .map_err(map_send)?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.map_err(map_send_mid_stream)?;
            if status.as_u16() == 404 {
                return Err(Error::ModelNotFound(body.model.clone()));
            }
            return Err(match super::error_message(&text) {
                Some(m) => Error::Ollama(m),
                None => Error::Http {
                    status: status.as_u16(),
                    body: text.chars().take(500).collect(),
                },
            });
        }
        Ok(ChatStream {
            resp,
            buf: Vec::new(),
            pending: VecDeque::new(),
            stall: self.chat_stall,
            first_event: self.first_event,
            seen_line: false,
            finished: false,
        })
    }

    /// Overrides the wait for the first event of a chat (model loading).
    pub fn with_first_event_timeout(mut self, d: Duration) -> OllamaClient {
        self.first_event = d;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::context::WireMessage;
    use crate::test_server::{Step, TcpScript};
    use serde_json::json;
    use std::time::Duration;

    fn line(content: &str, thinking: &str) -> Vec<u8> {
        let mut m = json!({"role": "assistant", "content": content});
        if !thinking.is_empty() {
            m["thinking"] = json!(thinking);
        }
        format!("{}\n", json!({"message": m, "done": false})).into_bytes()
    }

    fn done_line() -> Vec<u8> {
        b"{\"message\":{\"role\":\"assistant\",\"content\":\"\"},\"done\":true,\"done_reason\":\"stop\",\"prompt_eval_count\":42,\"eval_count\":7}\n".to_vec()
    }

    fn body() -> ChatRequestBody {
        ChatRequestBody {
            model: "llama3:8b".into(),
            messages: vec![WireMessage {
                role: "user".into(),
                content: "hi".into(),
            }],
            num_ctx: 4096,
            temperature: None,
            seed: None,
            think: None,
        }
    }

    fn fast(url: &str) -> OllamaClient {
        OllamaClient::new(url)
            .with_stall_timeout(Duration::from_millis(150))
            .with_first_event_timeout(Duration::from_millis(150))
    }

    async fn drain(mut s: ChatStream) -> (Vec<StreamEvent>, Option<Error>) {
        let mut out = Vec::new();
        loop {
            match s.next().await {
                Ok(Some(e)) => out.push(e),
                Ok(None) => return (out, None),
                Err(e) => return (out, Some(e)),
            }
        }
    }

    async fn run(steps: Vec<Step>) -> (Vec<StreamEvent>, Option<Error>) {
        let srv = TcpScript::start("200 OK", "", steps).await;
        let c = fast(&srv.base_url());
        let s = c.chat_stream(&body()).await.unwrap();
        drain(s).await
    }

    #[tokio::test]
    async fn content_and_thinking_deltas_arrive_in_order() {
        let (ev, err) = run(vec![
            Step::Write(line("", "hmm")),
            Step::Write(line("Hel", "")),
            Step::Write(line("lo", "")),
            Step::Write(done_line()),
            Step::Close,
        ])
        .await;
        assert!(err.is_none());
        assert_eq!(ev[0], StreamEvent::Thinking("hmm".into()));
        assert_eq!(ev[1], StreamEvent::Content("Hel".into()));
        assert_eq!(ev[2], StreamEvent::Content("lo".into()));
        assert!(matches!(ev[3], StreamEvent::Done { .. }));
        assert_eq!(ev.len(), 4);
    }

    #[tokio::test]
    async fn a_line_split_across_chunks_is_reassembled() {
        let l = line("whole", "");
        let (a, b) = l.split_at(20);
        let (ev, err) = run(vec![
            Step::Write(a.to_vec()),
            Step::Write(b.to_vec()),
            Step::Write(done_line()),
            Step::Close,
        ])
        .await;
        assert!(err.is_none());
        assert_eq!(ev[0], StreamEvent::Content("whole".into()));
    }

    #[tokio::test]
    async fn a_multibyte_character_split_across_chunks_decodes_intact() {
        let l = line("café 🦜", "");
        let at = l.iter().position(|&b| b == 0xC3).unwrap() + 1; // between C3 and A9
        let (a, b) = l.split_at(at);
        let (ev, err) = run(vec![
            Step::Write(a.to_vec()),
            Step::Write(b.to_vec()),
            Step::Write(done_line()),
            Step::Close,
        ])
        .await;
        assert!(err.is_none());
        assert_eq!(ev[0], StreamEvent::Content("café 🦜".into()));
    }

    #[tokio::test]
    async fn several_lines_in_one_chunk_yield_several_events() {
        let mut one = line("a", "");
        one.extend(line("b", ""));
        one.extend(done_line());
        let (ev, err) = run(vec![Step::Write(one), Step::Close]).await;
        assert!(err.is_none());
        assert_eq!(ev.len(), 3);
        assert_eq!(ev[1], StreamEvent::Content("b".into()));
    }

    #[tokio::test]
    async fn done_carries_the_counts_and_reason() {
        let (ev, _) = run(vec![Step::Write(done_line()), Step::Close]).await;
        assert_eq!(
            ev[0],
            StreamEvent::Done {
                prompt_eval_count: Some(42),
                eval_count: Some(7),
                done_reason: Some("stop".into())
            }
        );
    }

    #[tokio::test]
    async fn the_request_body_has_stream_true_and_num_ctx() {
        let srv = TcpScript::start("200 OK", "", vec![Step::Write(done_line()), Step::Close]).await;
        let mut b = body();
        b.temperature = Some(0.5);
        let s = fast(&srv.base_url()).chat_stream(&b).await.unwrap();
        let _ = drain(s).await;
        let sent: serde_json::Value = serde_json::from_str(&srv.request_body().await).unwrap();
        assert_eq!(sent["model"], "llama3:8b");
        assert_eq!(sent["stream"], true);
        assert_eq!(sent["options"]["num_ctx"], 4096);
        assert_eq!(sent["options"]["temperature"], 0.5);
        assert!(sent["options"].get("seed").is_none());
        assert!(sent.get("think").is_none());
        assert_eq!(sent["messages"][0]["role"], "user");
    }

    #[tokio::test]
    async fn a_404_becomes_model_not_found() {
        let srv = TcpScript::start(
            "404 Not Found",
            "",
            vec![
                Step::Write(b"{\"error\":\"model 'llama3:8b' not found\"}".to_vec()),
                Step::Close,
            ],
        )
        .await;
        let err = fast(&srv.base_url())
            .chat_stream(&body())
            .await
            .unwrap_err();
        assert_eq!(err, Error::ModelNotFound("llama3:8b".into()));
    }

    #[tokio::test]
    async fn a_500_json_error_becomes_an_ollama_error() {
        let srv = TcpScript::start(
            "500 Internal Server Error",
            "",
            vec![
                Step::Write(b"{\"error\":\"out of memory\"}".to_vec()),
                Step::Close,
            ],
        )
        .await;
        let err = fast(&srv.base_url())
            .chat_stream(&body())
            .await
            .unwrap_err();
        assert_eq!(err, Error::Ollama("out of memory".into()));
    }

    #[tokio::test]
    async fn an_in_stream_error_line_ends_with_an_ollama_error_after_earlier_events() {
        let (ev, err) = run(vec![
            Step::Write(line("part", "")),
            Step::Write(b"{\"error\":\"boom\"}\n".to_vec()),
            Step::Close,
        ])
        .await;
        assert_eq!(ev, vec![StreamEvent::Content("part".into())]);
        assert_eq!(err, Some(Error::Ollama("boom".into())));
    }

    #[tokio::test]
    async fn a_malformed_line_is_a_parse_error() {
        let (_, err) = run(vec![Step::Write(b"not json\n".to_vec()), Step::Close]).await;
        assert!(matches!(err, Some(Error::Parse(_))));
    }

    #[tokio::test]
    async fn closing_without_done_is_a_connection_ended_early_error() {
        let (ev, err) = run(vec![Step::Write(line("half", "")), Step::Close]).await;
        assert_eq!(ev, vec![StreamEvent::Content("half".into())]);
        assert_eq!(err, Some(Error::Network("connection ended early".into())));
    }

    #[tokio::test]
    async fn a_dropped_connection_mid_stream_is_a_network_error() {
        let (ev, err) = run(vec![Step::Write(line("half", ""))]).await;
        assert_eq!(ev, vec![StreamEvent::Content("half".into())]);
        assert!(matches!(err, Some(Error::Network(_))));
    }

    #[tokio::test]
    async fn a_stall_after_the_first_event_times_out() {
        let (ev, err) = run(vec![
            Step::Write(line("x", "")),
            Step::Pause(Duration::from_secs(5)),
        ])
        .await;
        assert_eq!(ev.len(), 1);
        assert_eq!(err, Some(Error::Network("ollama stalled".into())));
    }

    #[tokio::test]
    async fn no_first_event_times_out_with_a_loading_message() {
        let (ev, err) = run(vec![Step::Pause(Duration::from_secs(5))]).await;
        assert!(ev.is_empty());
        let Some(Error::Network(m)) = err else {
            panic!("expected a network error, got {err:?}")
        };
        assert!(m.contains("loading"), "{m}");
    }
}
