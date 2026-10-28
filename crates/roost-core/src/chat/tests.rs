use super::manager::{ChatConfig, ChatHandle, ChatManager};
use super::types::{ChatCommand, ChatEvent, SendRejection, TurnOutcome};
use crate::ollama::OllamaClient;
use crate::session::{
    AssistantStatus, MessageContent, Session, SessionDocument, SessionId, SessionLifetime,
};
use crate::store::{Store, Writer};
use crate::test_server::{Route, Step, TcpScript};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tempfile::TempDir;
use tokio::sync::oneshot;

type Events = Arc<Mutex<Vec<ChatEvent>>>;

struct H {
    handle: ChatHandle,
    events: Events,
    dir: TempDir,
}

const SHOW: &str = r#"{"details":{"family":"llama"},"model_info":{"llama.context_length":4096},"capabilities":["completion"],"digest":"sha256:abc"}"#;

fn show_route() -> Route {
    Route::new(
        "/api/show",
        "200 OK",
        "Content-Type: application/json\r\n",
        vec![Step::Write(SHOW.as_bytes().to_vec()), Step::Close],
    )
}

fn chat_route(steps: Vec<Step>) -> Route {
    Route::new("/api/chat", "200 OK", "", steps)
}

fn line(content: &str) -> Vec<u8> {
    format!(
        "{}\n",
        serde_json::json!({"message": {"role": "assistant", "content": content}, "done": false})
    )
    .into_bytes()
}

fn done() -> Vec<u8> {
    b"{\"message\":{\"role\":\"assistant\",\"content\":\"\"},\"done\":true,\"done_reason\":\"stop\",\"prompt_eval_count\":9}\n".to_vec()
}

/// A complete reply: `text`, then done, then a clean close.
fn reply(text: &str) -> Vec<Step> {
    vec![Step::Write(line(text)), Step::Write(done()), Step::Close]
}

async fn fake(routes: Vec<Route>) -> TcpScript {
    let mut all = vec![show_route()];
    all.extend(routes);
    TcpScript::start_routes(all).await
}

fn client(srv: &TcpScript) -> OllamaClient {
    OllamaClient::new(&srv.base_url())
}

async fn start(srv: &TcpScript) -> H {
    start_in(TempDir::new().unwrap(), client(srv), |_| {}).await
}

async fn start_in(dir: TempDir, c: OllamaClient, tweak: impl FnOnce(&mut ChatConfig)) -> H {
    let deadline = Instant::now() + Duration::from_secs(5);
    let (store, warnings) = loop {
        match Store::open(dir.path()) {
            Ok(v) => break v,
            Err(_) if Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(20)).await
            }
            Err(e) => panic!("store never opened: {e}"),
        }
    };
    let writer = Writer::spawn(store, warnings);
    let events: Events = Arc::default();
    let sink_events = Arc::clone(&events);
    let mut cfg = ChatConfig::new(c);
    cfg.coalesce = Duration::from_millis(5);
    tweak(&mut cfg);
    let handle = ChatManager::spawn(
        &tokio::runtime::Handle::current(),
        writer,
        cfg,
        Arc::new(move |e| sink_events.lock().unwrap().push(e)),
    );
    H {
        handle,
        events,
        dir,
    }
}

impl H {
    fn cmd(&self, c: ChatCommand) {
        self.handle.send(c);
    }

    fn events(&self) -> Vec<ChatEvent> {
        self.events.lock().unwrap().clone()
    }

    async fn wait(&self, what: &str, pred: impl Fn(&[ChatEvent]) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let snapshot = self.events();
            if pred(&snapshot) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}; events: {:#?}",
                self.events()
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    async fn create(&self, lifetime: SessionLifetime) -> SessionId {
        let before = self.opened().len();
        self.cmd(ChatCommand::Create {
            lifetime,
            model: "llama3:8b".into(),
        });
        self.wait("Opened", |e| opened(e).len() > before).await;
        *self.opened().last().unwrap()
    }

    fn opened(&self) -> Vec<SessionId> {
        opened(&self.events())
    }

    async fn send(&self, id: SessionId, text: &str) {
        self.cmd(ChatCommand::Send {
            session: id,
            text: text.into(),
        });
    }

    async fn wait_ended(&self, n: usize) {
        self.wait("TurnEnded", |e| ended(e).len() >= n).await;
    }

    fn deltas(&self, id: SessionId) -> String {
        self.events()
            .iter()
            .filter_map(|e| match e {
                ChatEvent::Delta {
                    session, content, ..
                } if *session == id => Some(content.clone()),
                _ => None,
            })
            .collect()
    }

    fn read(&self, id: SessionId) -> Session {
        read_session(self.dir.path(), id)
    }

    fn file(&self, id: SessionId) -> std::path::PathBuf {
        self.dir.path().join(format!("{id}.json"))
    }
}

fn opened(e: &[ChatEvent]) -> Vec<SessionId> {
    e.iter()
        .filter_map(|e| match e {
            ChatEvent::Opened(s) => Some(s.id),
            _ => None,
        })
        .collect()
}

fn ended(e: &[ChatEvent]) -> Vec<(TurnOutcome, Option<String>)> {
    e.iter()
        .filter_map(|e| match e {
            ChatEvent::TurnEnded {
                outcome, message, ..
            } => Some((*outcome, message.clone())),
            _ => None,
        })
        .collect()
}

fn last_rows(e: &[ChatEvent]) -> Vec<crate::store::IndexRow> {
    e.iter()
        .rev()
        .find_map(|e| match e {
            ChatEvent::Rows(r) => Some(r.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

fn read_session(dir: &Path, id: SessionId) -> Session {
    let bytes = std::fs::read(dir.join(format!("{id}.json"))).unwrap();
    serde_json::from_slice::<SessionDocument>(&bytes)
        .unwrap()
        .session
}

fn preload(dir: &Path, s: &Session) {
    let (mut st, _) = Store::open(dir).unwrap();
    st.save(s).unwrap();
}

fn texts(s: &Session) -> Vec<(String, String)> {
    s.messages
        .iter()
        .map(|m| match &m.content {
            MessageContent::User { text, .. } => ("user".into(), text.clone()),
            MessageContent::Assistant { text, status, .. } => {
                (format!("assistant:{status:?}"), text.clone())
            }
            MessageContent::Tool { .. } => ("tool".into(), String::new()),
        })
        .collect()
}

fn chat_bodies(srv: &TcpScript) -> Vec<serde_json::Value> {
    srv.requests()
        .into_iter()
        .filter(|(p, _)| p == "/api/chat")
        .map(|(_, b)| serde_json::from_str(&b).unwrap())
        .collect()
}

fn dir_snapshot(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut v: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .filter(|e| e.file_name() != ".lock")
        .map(|e| {
            (
                e.file_name().to_string_lossy().into_owned(),
                std::fs::read(e.path()).unwrap_or_default(),
            )
        })
        .collect();
    v.sort();
    v
}

// ---------------------------------------------------------------- lifecycle

#[tokio::test]
async fn create_persisted_chat_appears_in_rows_and_on_disk() {
    let srv = fake(vec![]).await;
    let h = start(&srv).await;
    let id = h.create(SessionLifetime::Persisted).await;
    h.wait("rows", |e| last_rows(e).iter().any(|r| r.id == id))
        .await;
    assert!(h.file(id).exists());
}

#[tokio::test]
async fn create_incognito_appears_in_rows_but_writes_nothing_to_disk() {
    let srv = fake(vec![]).await;
    let h = start(&srv).await;
    let before = dir_snapshot(h.dir.path());
    let id = h.create(SessionLifetime::Incognito).await;
    h.wait("rows", |e| last_rows(e).iter().any(|r| r.id == id))
        .await;
    let row = last_rows(&h.events())
        .into_iter()
        .find(|r| r.id == id)
        .unwrap();
    assert_eq!(row.lifetime, SessionLifetime::Incognito);
    assert_eq!(dir_snapshot(h.dir.path()), before);
}

#[tokio::test]
async fn open_returns_the_transcript() {
    let srv = fake(vec![chat_route(reply("Hello"))]).await;
    let h = start(&srv).await;
    let id = h.create(SessionLifetime::Persisted).await;
    h.send(id, "hi").await;
    h.wait_ended(1).await;
    h.cmd(ChatCommand::Close(id));
    h.cmd(ChatCommand::Open(id));
    h.wait("reopen", |e| opened(e).len() == 2).await;
    let reopened = h
        .events()
        .into_iter()
        .rev()
        .find_map(|e| match e {
            ChatEvent::Opened(s) => Some(s),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        texts(&reopened),
        vec![
            ("user".into(), "hi".into()),
            ("assistant:Complete".into(), "Hello".into())
        ]
    );
}

#[tokio::test]
async fn rename_and_pin_do_not_change_updated_at_or_order() {
    let srv = fake(vec![]).await;
    let h = start(&srv).await;
    let a = h.create(SessionLifetime::Persisted).await;
    tokio::time::sleep(Duration::from_millis(5)).await;
    let b = h.create(SessionLifetime::Persisted).await;
    h.wait("rows", |e| last_rows(e).len() == 2).await;
    let before = last_rows(&h.events());
    h.cmd(ChatCommand::Rename(a, "Renamed".into()));
    h.cmd(ChatCommand::SetPinned(a, true));
    h.wait("rename+pin", |e| {
        last_rows(e)
            .iter()
            .any(|r| r.id == a && r.pinned && r.title == "Renamed")
    })
    .await;
    let after = last_rows(&h.events());
    let upd = |rows: &[crate::store::IndexRow], id| {
        rows.iter().find(|r| r.id == id).unwrap().updated_at_ms
    };
    assert_eq!(upd(&before, a), upd(&after, a));
    assert_eq!(
        after.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![b, a],
        "order must not change"
    );
}

#[tokio::test]
async fn closing_incognito_destroys_it() {
    let srv = fake(vec![]).await;
    let h = start(&srv).await;
    let id = h.create(SessionLifetime::Incognito).await;
    h.cmd(ChatCommand::Close(id));
    h.wait("gone", |e| !last_rows(e).iter().any(|r| r.id == id))
        .await;
    h.cmd(ChatCommand::Open(id));
    h.wait("error", |e| {
        e.iter()
            .any(|e| matches!(e, ChatEvent::SessionError { session: Some(s), .. } if *s == id))
    })
    .await;
}

#[tokio::test]
async fn clear_incognito_destroys_all_and_keeps_saved_chats() {
    let srv = fake(vec![]).await;
    let h = start(&srv).await;
    let saved = h.create(SessionLifetime::Persisted).await;
    let i1 = h.create(SessionLifetime::Incognito).await;
    let i2 = h.create(SessionLifetime::Incognito).await;
    h.cmd(ChatCommand::ClearIncognito);
    h.wait("cleared", |e| {
        let ids: Vec<_> = last_rows(e).iter().map(|r| r.id).collect();
        ids.contains(&saved) && !ids.contains(&i1) && !ids.contains(&i2)
    })
    .await;
}

#[tokio::test]
async fn delete_removes_file_and_row() {
    let srv = fake(vec![]).await;
    let h = start(&srv).await;
    let id = h.create(SessionLifetime::Persisted).await;
    h.cmd(ChatCommand::Delete(id));
    h.wait("deleted", |e| {
        !last_rows(e).iter().any(|r| r.id == id) && e.len() > 3
    })
    .await;
    assert!(!h.file(id).exists());
}

#[tokio::test]
async fn set_model_changes_model_when_idle() {
    let srv = fake(vec![]).await;
    let h = start(&srv).await;
    let id = h.create(SessionLifetime::Persisted).await;
    h.cmd(ChatCommand::SetModel(id, "qwen:7b".into()));
    h.wait("model", |e| {
        last_rows(e)
            .iter()
            .any(|r| r.id == id && r.model == "qwen:7b")
    })
    .await;
}

#[tokio::test]
async fn restart_lists_saved_sessions_again() {
    let srv = fake(vec![]).await;
    let h = start(&srv).await;
    let id = h.create(SessionLifetime::Persisted).await;
    h.wait("rows", |e| last_rows(e).iter().any(|r| r.id == id))
        .await;
    h.handle.shutdown().await;
    let H { dir, .. } = h;
    let h2 = start_in(dir, client(&srv), |_| {}).await;
    h2.wait("rows", |e| last_rows(e).iter().any(|r| r.id == id))
        .await;
}

// ---------------------------------------------------------------- runs

#[tokio::test]
async fn a_chat_streams_and_completes_and_is_saved() {
    let srv = fake(vec![chat_route(vec![
        Step::Write(line("Hel")),
        Step::Write(line("lo")),
        Step::Write(done()),
        Step::Close,
    ])])
    .await;
    let h = start(&srv).await;
    let id = h.create(SessionLifetime::Persisted).await;
    h.send(id, "hi").await;
    h.wait_ended(1).await;
    assert_eq!(ended(&h.events())[0].0, TurnOutcome::Completed);
    assert_eq!(h.deltas(id), "Hello");
    h.wait("saved", |_| {
        texts(&h.read(id)).last().is_some_and(|m| m.1 == "Hello")
    })
    .await;
    assert_eq!(
        texts(&h.read(id)),
        vec![
            ("user".into(), "hi".into()),
            ("assistant:Complete".into(), "Hello".into())
        ]
    );
}

#[tokio::test]
async fn the_request_carries_num_ctx_equal_to_the_trim_window() {
    let srv = fake(vec![chat_route(reply("ok"))]).await;
    let h = start(&srv).await;
    let id = h.create(SessionLifetime::Persisted).await;
    h.send(id, "hi").await;
    h.wait_ended(1).await;
    let bodies = chat_bodies(&srv);
    assert_eq!(bodies[0]["options"]["num_ctx"], 4096);
    assert_eq!(bodies[0]["messages"][0]["role"], "system");
    assert_eq!(bodies[0]["messages"][1]["content"], "hi");
}

#[tokio::test]
async fn a_second_send_while_running_is_rejected_busy() {
    let (release, hold) = oneshot::channel();
    let srv = fake(vec![chat_route(vec![
        Step::Write(line("a")),
        Step::WaitFor(hold),
        Step::Write(done()),
        Step::Close,
    ])])
    .await;
    let h = start(&srv).await;
    let id = h.create(SessionLifetime::Persisted).await;
    h.send(id, "one").await;
    h.wait("streaming", |_| !h.deltas(id).is_empty()).await;
    h.send(id, "two").await;
    h.wait("rejected", |e| {
        e.iter().any(|e| {
            matches!(
                e,
                ChatEvent::SendRejected {
                    reason: SendRejection::Busy,
                    ..
                }
            )
        })
    })
    .await;
    release.send(()).unwrap();
    h.wait_ended(1).await;
    assert_eq!(
        texts(&h.read(id)).iter().filter(|m| m.0 == "user").count(),
        1
    );
}

#[tokio::test]
async fn stop_cancels_and_keeps_the_partial_text_labelled_cancelled() {
    let (_keep, hold) = oneshot::channel::<()>();
    let srv = fake(vec![chat_route(vec![
        Step::Write(line("par")),
        Step::WaitFor(hold),
    ])])
    .await;
    let h = start(&srv).await;
    let id = h.create(SessionLifetime::Persisted).await;
    h.send(id, "go").await;
    h.wait("partial", |_| h.deltas(id) == "par").await;
    h.cmd(ChatCommand::Stop(id));
    h.wait_ended(1).await;
    assert_eq!(ended(&h.events())[0].0, TurnOutcome::Cancelled);
    h.wait("saved", |_| {
        texts(&h.read(id))
            .last()
            .is_some_and(|m| m.0 == "assistant:Cancelled")
    })
    .await;
    assert_eq!(texts(&h.read(id)).last().unwrap().1, "par");
}

#[tokio::test]
async fn stop_racing_a_held_done_line_yields_one_terminal_cancelled() {
    let (release, hold) = oneshot::channel();
    let srv = fake(vec![chat_route(vec![
        Step::Write(line("a")),
        Step::WaitFor(hold),
        Step::Write(done()),
        Step::Close,
    ])])
    .await;
    let h = start(&srv).await;
    let id = h.create(SessionLifetime::Persisted).await;
    h.send(id, "go").await;
    h.wait("partial", |_| h.deltas(id) == "a").await;
    h.cmd(ChatCommand::Stop(id));
    h.wait_ended(1).await;
    let seen = h.events().len();
    let _ = release.send(());
    tokio::time::sleep(Duration::from_millis(150)).await;
    let all = h.events();
    assert_eq!(ended(&all).len(), 1);
    assert_eq!(ended(&all)[0].0, TurnOutcome::Cancelled);
    assert!(
        !all[seen..]
            .iter()
            .any(|e| matches!(e, ChatEvent::Delta { .. } | ChatEvent::TurnEnded { .. })),
        "events after the terminal one: {:?}",
        &all[seen..]
    );
}

#[tokio::test]
async fn completion_then_stop_yields_one_terminal_completed() {
    let srv = fake(vec![chat_route(reply("done"))]).await;
    let h = start(&srv).await;
    let id = h.create(SessionLifetime::Persisted).await;
    h.send(id, "go").await;
    h.wait_ended(1).await;
    h.cmd(ChatCommand::Stop(id));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(ended(&h.events()), vec![(TurnOutcome::Completed, None)]);
}

#[tokio::test]
async fn early_close_without_done_fails_and_keeps_partial_text() {
    let srv = fake(vec![chat_route(vec![
        Step::Write(line("half")),
        Step::Close,
    ])])
    .await;
    let h = start(&srv).await;
    let id = h.create(SessionLifetime::Persisted).await;
    h.send(id, "go").await;
    h.wait_ended(1).await;
    let (outcome, msg) = ended(&h.events())[0].clone();
    assert_eq!(outcome, TurnOutcome::Failed);
    assert!(msg.unwrap().contains("connection ended early"));
    h.wait("saved", |_| {
        texts(&h.read(id))
            .last()
            .is_some_and(|m| m.0 == "assistant:Failed")
    })
    .await;
    assert_eq!(texts(&h.read(id)).last().unwrap().1, "half");
}

#[tokio::test]
async fn a_stall_fails_once() {
    let srv = fake(vec![chat_route(vec![
        Step::Write(line("x")),
        Step::Pause(Duration::from_secs(5)),
    ])])
    .await;
    let c = client(&srv).with_stall_timeout(Duration::from_millis(150));
    let h = start_in(TempDir::new().unwrap(), c, |_| {}).await;
    let id = h.create(SessionLifetime::Persisted).await;
    h.send(id, "go").await;
    h.wait_ended(1).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let all = ended(&h.events());
    assert_eq!(all.len(), 1);
    assert!(all[0].1.as_ref().unwrap().contains("stopped responding"));
}

#[tokio::test]
async fn unreachable_ollama_fails_once_with_a_readable_message() {
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let c = OllamaClient::new(&format!("http://127.0.0.1:{port}"));
    let h = start_in(TempDir::new().unwrap(), c, |_| {}).await;
    let id = h.create(SessionLifetime::Persisted).await;
    h.send(id, "go").await;
    h.wait_ended(1).await;
    let all = ended(&h.events());
    assert_eq!(all.len(), 1);
    assert!(all[0].1.as_ref().unwrap().contains("not reachable"));
}

#[tokio::test]
async fn failed_before_the_first_token_leaves_an_empty_error_message() {
    let srv = fake(vec![Route::new(
        "/api/chat",
        "500 Internal Server Error",
        "",
        vec![
            Step::Write(b"{\"error\":\"out of memory\"}".to_vec()),
            Step::Close,
        ],
    )])
    .await;
    let h = start(&srv).await;
    let id = h.create(SessionLifetime::Persisted).await;
    h.send(id, "go").await;
    h.wait_ended(1).await;
    h.wait("saved", |_| {
        texts(&h.read(id))
            .get(1)
            .is_some_and(|m| m.0 == "assistant:Failed")
    })
    .await;
    let s = h.read(id);
    assert!(matches!(
        &s.messages[1].content,
        MessageContent::Assistant { text, status: AssistantStatus::Failed, error: Some(e), .. }
            if text.is_empty() && e.contains("out of memory")
    ));
}

#[tokio::test]
async fn a_404_emits_model_missing() {
    let srv = fake(vec![Route::new(
        "/api/chat",
        "404 Not Found",
        "",
        vec![
            Step::Write(b"{\"error\":\"model not found\"}".to_vec()),
            Step::Close,
        ],
    )])
    .await;
    let h = start(&srv).await;
    let id = h.create(SessionLifetime::Persisted).await;
    h.send(id, "go").await;
    h.wait_ended(1).await;
    assert!(h
        .events()
        .iter()
        .any(|e| matches!(e, ChatEvent::ModelMissing { model, .. } if model == "llama3:8b")));
}

#[tokio::test]
async fn retry_after_failure_appends_a_new_reply_and_excludes_the_failed_one_from_context() {
    let srv = fake(vec![
        chat_route(vec![Step::Write(line("half")), Step::Close]),
        chat_route(reply("ok")),
    ])
    .await;
    let h = start(&srv).await;
    let id = h.create(SessionLifetime::Persisted).await;
    h.send(id, "ask").await;
    h.wait_ended(1).await;
    h.cmd(ChatCommand::Retry(id));
    h.wait_ended(2).await;
    assert_eq!(ended(&h.events())[1].0, TurnOutcome::Completed);
    h.wait("saved", |_| {
        texts(&h.read(id)).len() == 3 && texts(&h.read(id))[2].1 == "ok"
    })
    .await;
    assert_eq!(
        texts(&h.read(id))[1],
        ("assistant:Failed".into(), "half".into())
    );
    let second = &chat_bodies(&srv)[1]["messages"];
    assert_eq!(second.as_array().unwrap().len(), 2, "{second}");
    assert_eq!(second[1]["content"], "ask");
}

#[tokio::test]
async fn a_crash_interrupted_last_turn_offers_retry_and_resends() {
    let dir = TempDir::new().unwrap();
    let mut s = Session::new_chat("llama3:8b", SessionLifetime::Persisted, 10);
    s.push_user("question", 11);
    let a = s.begin_assistant(
        crate::session::RunId::new(),
        crate::session::TurnId::new(),
        12,
    );
    s.append_assistant(a, "part", "");
    preload(dir.path(), &s);
    let srv = fake(vec![chat_route(reply("answer"))]).await;
    let h = start_in(dir, client(&srv), |_| {}).await;
    h.cmd(ChatCommand::Retry(s.id));
    h.wait_ended(1).await;
    assert_eq!(ended(&h.events())[0].0, TurnOutcome::Completed);
    assert_eq!(chat_bodies(&srv)[0]["messages"][1]["content"], "question");
}

#[tokio::test]
async fn a_new_message_after_a_failed_exchange_sends_no_consecutive_user_turns() {
    let srv = fake(vec![
        chat_route(vec![Step::Write(line("half")), Step::Close]),
        chat_route(reply("fine")),
    ])
    .await;
    let h = start(&srv).await;
    let id = h.create(SessionLifetime::Persisted).await;
    h.send(id, "first").await;
    h.wait_ended(1).await;
    h.send(id, "second").await;
    h.wait_ended(2).await;
    let roles: Vec<String> = chat_bodies(&srv)[1]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["role"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(roles, ["system", "user"]);
    assert_eq!(chat_bodies(&srv)[1]["messages"][1]["content"], "second");
}

#[tokio::test]
async fn long_history_is_trimmed_with_the_marker_and_the_transcript_is_untouched() {
    let dir = TempDir::new().unwrap();
    let mut s = Session::new_chat("llama3:8b", SessionLifetime::Persisted, 10);
    for i in 0..40 {
        s.push_user(&format!("q{i} {}", "x".repeat(600)), 11);
        let a = s.begin_assistant(
            crate::session::RunId::new(),
            crate::session::TurnId::new(),
            12,
        );
        s.append_assistant(a, &"y".repeat(600), "");
        s.finish_assistant(a, AssistantStatus::Complete, None, 13);
    }
    preload(dir.path(), &s);
    let srv = fake(vec![chat_route(reply("ok"))]).await;
    let h = start_in(dir, client(&srv), |_| {}).await;
    h.send(s.id, "latest").await;
    h.wait_ended(1).await;
    assert!(h
        .events()
        .iter()
        .any(|e| matches!(e, ChatEvent::ContextTrimmed { omitted, .. } if *omitted > 0)));
    let sent = chat_bodies(&srv)[0]["messages"].as_array().unwrap().clone();
    assert_eq!(sent[1]["content"], "Earlier messages omitted for context");
    assert!(sent.len() < 80);
    h.wait("saved", |_| h.read(s.id).messages.len() == 82).await;
}

#[tokio::test]
async fn an_oversized_message_is_rejected_too_large_and_nothing_is_sent_or_saved() {
    let srv = fake(vec![chat_route(reply("never"))]).await;
    let h = start(&srv).await;
    let id = h.create(SessionLifetime::Persisted).await;
    h.send(id, &"x".repeat(30_000)).await;
    h.wait("rejected", |e| {
        e.iter().any(|e| {
            matches!(
                e,
                ChatEvent::SendRejected {
                    reason: SendRejection::TooLarge(_),
                    ..
                }
            )
        })
    })
    .await;
    assert!(chat_bodies(&srv).is_empty());
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(h.read(id).messages.is_empty());
}

#[tokio::test]
async fn deltas_are_coalesced_and_arrive_before_the_terminal_event() {
    let mut steps: Vec<Step> = (0..20)
        .map(|i| Step::Write(line(&format!("{i},"))))
        .collect();
    steps.push(Step::Write(done()));
    steps.push(Step::Close);
    let srv = fake(vec![chat_route(steps)]).await;
    let c = client(&srv);
    let h = start_in(TempDir::new().unwrap(), c, |cfg| {
        cfg.coalesce = Duration::from_millis(80)
    })
    .await;
    let id = h.create(SessionLifetime::Persisted).await;
    h.send(id, "go").await;
    h.wait_ended(1).await;
    let events = h.events();
    let delta_count = events
        .iter()
        .filter(|e| matches!(e, ChatEvent::Delta { .. }))
        .count();
    assert!(delta_count < 20, "{delta_count} deltas");
    let last_delta = events
        .iter()
        .rposition(|e| matches!(e, ChatEvent::Delta { .. }))
        .unwrap();
    let end = events
        .iter()
        .position(|e| matches!(e, ChatEvent::TurnEnded { .. }))
        .unwrap();
    assert!(last_delta < end);
    let expected: String = (0..20).map(|i| format!("{i},")).collect();
    assert_eq!(h.deltas(id), expected);
}

#[tokio::test]
async fn partial_text_is_flushed_to_disk_while_streaming() {
    let (release, hold) = oneshot::channel();
    let srv = fake(vec![chat_route(vec![
        Step::Write(line("par")),
        Step::WaitFor(hold),
        Step::Write(done()),
        Step::Close,
    ])])
    .await;
    let h = start_in(TempDir::new().unwrap(), client(&srv), |cfg| {
        cfg.flush_interval = Duration::from_millis(40)
    })
    .await;
    let id = h.create(SessionLifetime::Persisted).await;
    h.send(id, "go").await;
    h.wait("flushed", |_| {
        std::fs::read(h.file(id))
            .ok()
            .and_then(|b| serde_json::from_slice::<SessionDocument>(&b).ok())
            .is_some_and(|d| texts(&d.session).last().is_some_and(|m| m.1 == "par"))
    })
    .await;
    release.send(()).unwrap();
    h.wait_ended(1).await;
}

#[tokio::test]
async fn two_sessions_run_concurrently() {
    let (r1, h1) = oneshot::channel();
    let (r2, h2) = oneshot::channel();
    let srv = fake(vec![
        chat_route(vec![
            Step::Write(line("a")),
            Step::WaitFor(h1),
            Step::Write(done()),
            Step::Close,
        ]),
        chat_route(vec![
            Step::Write(line("b")),
            Step::WaitFor(h2),
            Step::Write(done()),
            Step::Close,
        ]),
    ])
    .await;
    let h = start(&srv).await;
    let s1 = h.create(SessionLifetime::Persisted).await;
    let s2 = h.create(SessionLifetime::Persisted).await;
    h.send(s1, "one").await;
    h.wait("first streaming", |_| !h.deltas(s1).is_empty())
        .await;
    h.send(s2, "two").await;
    h.wait("second streaming", |_| !h.deltas(s2).is_empty())
        .await;
    assert!(
        ended(&h.events()).is_empty(),
        "both must be running at once"
    );
    assert!(h
        .events()
        .iter()
        .any(|e| matches!(e, ChatEvent::WaitingForModel { session, .. } if *session == s2)));
    r1.send(()).unwrap();
    r2.send(()).unwrap();
    h.wait_ended(2).await;
}

// ---------------------------------------------------------------- teardown

#[tokio::test]
async fn delete_during_a_run_leaves_no_file_even_with_a_late_flush() {
    let (_keep, hold) = oneshot::channel::<()>();
    let srv = fake(vec![chat_route(vec![
        Step::Write(line("x")),
        Step::WaitFor(hold),
    ])])
    .await;
    let h = start_in(TempDir::new().unwrap(), client(&srv), |cfg| {
        cfg.flush_interval = Duration::from_millis(15)
    })
    .await;
    let id = h.create(SessionLifetime::Persisted).await;
    h.send(id, "go").await;
    h.wait("streaming", |_| !h.deltas(id).is_empty()).await;
    h.cmd(ChatCommand::Delete(id));
    h.wait("rows without it", |e| {
        e.iter()
            .rev()
            .take(3)
            .any(|e| matches!(e, ChatEvent::Rows(r) if r.iter().all(|x| x.id != id)))
    })
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!h.file(id).exists());
    assert!(
        ended(&h.events()).is_empty(),
        "a deleted session emits no terminal event"
    );
}

#[tokio::test]
async fn closing_a_running_saved_session_keeps_the_final_flush() {
    let (release, hold) = oneshot::channel();
    let srv = fake(vec![chat_route(vec![
        Step::Write(line("one ")),
        Step::WaitFor(hold),
        Step::Write(line("two")),
        Step::Write(done()),
        Step::Close,
    ])])
    .await;
    let h = start(&srv).await;
    let id = h.create(SessionLifetime::Persisted).await;
    h.send(id, "go").await;
    h.wait("streaming", |_| !h.deltas(id).is_empty()).await;
    h.cmd(ChatCommand::Close(id));
    release.send(()).unwrap();
    h.wait_ended(1).await;
    h.wait("saved", |_| {
        texts(&h.read(id)).last().is_some_and(|m| m.1 == "one two")
    })
    .await;
}

#[tokio::test]
async fn closing_a_running_incognito_session_cancels_and_destroys_it() {
    let (_keep, hold) = oneshot::channel::<()>();
    let srv = fake(vec![chat_route(vec![
        Step::Write(line("x")),
        Step::WaitFor(hold),
    ])])
    .await;
    let h = start(&srv).await;
    let id = h.create(SessionLifetime::Incognito).await;
    h.send(id, "go").await;
    h.wait("streaming", |_| !h.deltas(id).is_empty()).await;
    h.cmd(ChatCommand::Close(id));
    h.wait("gone", |e| !last_rows(e).iter().any(|r| r.id == id))
        .await;
}

#[tokio::test]
async fn shutdown_flushes_a_cancelled_partial_within_the_limit() {
    let (_keep, hold) = oneshot::channel::<()>();
    let srv = fake(vec![chat_route(vec![
        Step::Write(line("partial")),
        Step::WaitFor(hold),
    ])])
    .await;
    let h = start(&srv).await;
    let id = h.create(SessionLifetime::Persisted).await;
    h.send(id, "go").await;
    h.wait("streaming", |_| h.deltas(id) == "partial").await;
    let t = Instant::now();
    h.handle.shutdown().await;
    assert!(t.elapsed() < Duration::from_secs(3));
    let s = h.read(id);
    assert_eq!(
        texts(&s).last().unwrap(),
        &("assistant:Cancelled".to_string(), "partial".to_string())
    );
    assert!(s.is_idle());
}

#[tokio::test]
async fn a_full_incognito_conversation_leaves_the_data_dir_byte_identical() {
    let srv = fake(vec![chat_route(reply("secret reply"))]).await;
    let h = start(&srv).await;
    let before = dir_snapshot(h.dir.path());
    let id = h.create(SessionLifetime::Incognito).await;
    h.send(id, "secret question").await;
    h.wait_ended(1).await;
    h.cmd(ChatCommand::Rename(id, "secret title".into()));
    h.cmd(ChatCommand::SetPinned(id, true));
    h.cmd(ChatCommand::Close(id));
    h.cmd(ChatCommand::ClearIncognito);
    h.handle.shutdown().await;
    let after = dir_snapshot(h.dir.path());
    assert_eq!(after, before);
    for (name, bytes) in &after {
        assert!(!String::from_utf8_lossy(bytes).contains("secret"), "{name}");
    }
}

#[tokio::test]
async fn opening_a_running_session_flushes_pending_text_before_the_snapshot() {
    let (_keep, hold) = oneshot::channel::<()>();
    let srv = fake(vec![chat_route(vec![
        Step::Write(line("a")),
        Step::Write(line("b")),
        Step::WaitFor(hold),
    ])])
    .await;
    // A long batching window keeps the text pending when Open arrives.
    let h = start_in(TempDir::new().unwrap(), client(&srv), |cfg| {
        cfg.coalesce = Duration::from_secs(30)
    })
    .await;
    let id = h.create(SessionLifetime::Persisted).await;
    h.send(id, "go").await;
    h.wait("both chunks arrived", |_| {
        srv.requests().iter().any(|(p, _)| p == "/api/chat")
    })
    .await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    h.cmd(ChatCommand::Open(id));
    h.wait("reopened", |e| opened(e).len() == 2).await;
    let events = h.events();
    let at = events
        .iter()
        .rposition(|e| matches!(e, ChatEvent::Opened(_)))
        .unwrap();
    let ChatEvent::Opened(snapshot) = &events[at] else {
        unreachable!()
    };
    let snap_text = texts(snapshot).last().unwrap().1.clone();
    assert_eq!(snap_text, "ab");
    let flushed_before: String = events[..at]
        .iter()
        .filter_map(|e| match e {
            ChatEvent::Delta { content, .. } => Some(content.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        flushed_before, "ab",
        "pending text must be sent before the snapshot"
    );
}
