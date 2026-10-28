use super::manager::{ChatConfig, ChatHandle, ChatManager};
use super::types::{BranchState, ChatCommand, ChatEvent, SendRejection, TurnOutcome};
use crate::ollama::OllamaClient;
use crate::session::{
    AssistantStatus, MessageContent, MessageId, Session, SessionDocument, SessionId,
    SessionLifetime,
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

    /// `read`, but `None` while the file is not there yet.
    fn try_read(&self, id: SessionId) -> Option<Session> {
        try_read_session(self.dir.path(), id)
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

/// The saved chat. A file being rewritten at this moment is read again
/// shortly, so a test never trips over a half-written one.
fn read_session(dir: &Path, id: SessionId) -> Session {
    for _ in 0..200 {
        if let Some(s) = try_read_session(dir, id) {
            return s;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("the saved chat {id} could not be read");
}

/// The saved chat, or `None` while its file is not (completely) there yet.
fn try_read_session(dir: &Path, id: SessionId) -> Option<Session> {
    let bytes = std::fs::read(dir.join(format!("{id}.json"))).ok()?;
    serde_json::from_slice::<SessionDocument>(&bytes)
        .ok()
        .map(|d| d.session)
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
    // At every Open, the text flushed before the snapshot must equal the
    // snapshot's text. Keep opening until both chunks have arrived.
    let mut reached = false;
    for round in 1..=100 {
        h.cmd(ChatCommand::Open(id));
        h.wait("reopened", |e| opened(e).len() == round + 1).await;
        let events = h.events();
        let at = events
            .iter()
            .rposition(|e| matches!(e, ChatEvent::Opened(_)))
            .unwrap();
        let ChatEvent::Opened(snapshot) = &events[at] else {
            unreachable!()
        };
        let snap_text = texts(snapshot).last().unwrap().1.clone();
        let flushed_before: String = events[..at]
            .iter()
            .filter_map(|e| match e {
                ChatEvent::Delta { content, .. } => Some(content.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            flushed_before, snap_text,
            "pending text must be sent before the snapshot"
        );
        if snap_text == "ab" {
            reached = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    assert!(reached, "both chunks should have arrived");
}

#[tokio::test]
async fn a_reply_cut_off_by_the_length_limit_is_failed_not_completed() {
    let cut = b"{\"message\":{\"role\":\"assistant\",\"content\":\"\"},\"done\":true,\"done_reason\":\"length\"}\n".to_vec();
    let srv = fake(vec![chat_route(vec![
        Step::Write(line("half an ans")),
        Step::Write(cut),
        Step::Close,
    ])])
    .await;
    let h = start(&srv).await;
    let id = h.create(SessionLifetime::Persisted).await;
    h.send(id, "go").await;
    h.wait_ended(1).await;
    let (outcome, msg) = ended(&h.events())[0].clone();
    assert_eq!(outcome, TurnOutcome::Failed);
    assert!(msg.unwrap().contains("length limit"));
    h.wait("saved", |_| {
        texts(&h.read(id))
            .last()
            .is_some_and(|m| m.0 == "assistant:Failed")
    })
    .await;
    assert_eq!(texts(&h.read(id)).last().unwrap().1, "half an ans");
}

#[tokio::test]
async fn an_unchanged_transcript_is_not_rewritten_on_every_flush() {
    let (release, hold) = oneshot::channel();
    let srv = fake(vec![chat_route(vec![
        Step::Write(line("x")),
        Step::WaitFor(hold),
        Step::Write(done()),
        Step::Close,
    ])])
    .await;
    let h = start_in(TempDir::new().unwrap(), client(&srv), |cfg| {
        cfg.flush_interval = Duration::from_millis(20)
    })
    .await;
    let id = h.create(SessionLifetime::Persisted).await;
    h.send(id, "go").await;
    h.wait("flushed", |_| {
        std::fs::read(h.file(id)).is_ok_and(|b| String::from_utf8_lossy(&b).contains("\"x\""))
    })
    .await;
    // Nothing new is streaming now: the file must stop being rewritten.
    tokio::time::sleep(Duration::from_millis(100)).await;
    let stamp = std::fs::metadata(h.file(id)).unwrap().modified().unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        std::fs::metadata(h.file(id)).unwrap().modified().unwrap(),
        stamp
    );
    release.send(()).unwrap();
    h.wait_ended(1).await;
}

// ------------------------------------------------------------------ agents

const SHOW_TOOLS: &str = r#"{"details":{"family":"llama"},"model_info":{"llama.context_length":8192},"capabilities":["completion","tools"],"digest":"sha256:abc"}"#;

fn show_tools_route() -> Route {
    Route::new(
        "/api/show",
        "200 OK",
        "Content-Type: application/json\r\n",
        vec![Step::Write(SHOW_TOOLS.as_bytes().to_vec()), Step::Close],
    )
}

async fn fake_agent(routes: Vec<Route>) -> TcpScript {
    let mut all = vec![show_tools_route()];
    all.extend(routes);
    TcpScript::start_routes(all).await
}

fn tool_call(name: &str, args: serde_json::Value) -> Vec<u8> {
    format!(
        "{}\n",
        serde_json::json!({
            "message": {"role": "assistant", "content": "", "tool_calls": [{"function": {"name": name, "arguments": args}}]},
            "done": false
        })
    )
    .into_bytes()
}

/// A model step that asks for one tool.
fn calls(name: &str, args: serde_json::Value) -> Vec<Step> {
    vec![
        Step::Write(tool_call(name, args)),
        Step::Write(done()),
        Step::Close,
    ]
}

fn run_ends(e: &[ChatEvent]) -> Vec<(super::types::RunOutcome, Option<String>)> {
    e.iter()
        .filter_map(|e| match e {
            ChatEvent::RunEnded {
                outcome, message, ..
            } => Some((*outcome, message.clone())),
            _ => None,
        })
        .collect()
}

fn workspace() -> TempDir {
    let w = TempDir::new().unwrap();
    std::fs::write(w.path().join("a.txt"), "hello\nworld\n").unwrap();
    std::fs::write(w.path().join(".env"), "TOKEN=1").unwrap();
    w
}

impl H {
    async fn create_agent(&self, ws: &TempDir, lifetime: SessionLifetime) -> SessionId {
        let before = self.opened().len();
        self.cmd(ChatCommand::CreateAgent {
            lifetime,
            model: "llama3:8b".into(),
            workspace: ws.path().to_path_buf(),
        });
        self.wait("agent opened", |e| opened(e).len() > before)
            .await;
        *self.opened().last().unwrap()
    }

    async fn wait_run_ended(&self, n: usize) {
        self.wait("RunEnded", |e| run_ends(e).len() >= n).await;
    }
}

#[tokio::test]
async fn an_agent_reads_a_file_and_answers() {
    let ws = workspace();
    let srv = fake_agent(vec![
        chat_route(calls("read_file", serde_json::json!({"path": "a.txt"}))),
        chat_route(reply("It says hello.")),
    ])
    .await;
    let h = start(&srv).await;
    let id = h.create_agent(&ws, SessionLifetime::Persisted).await;
    h.send(id, "what is in a.txt?").await;
    h.wait_run_ended(1).await;
    let events = h.events();
    assert_eq!(
        run_ends(&events),
        vec![(super::types::RunOutcome::Completed, None)]
    );
    let started: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            ChatEvent::ToolStarted {
                name, arguments, ..
            } => Some((name.clone(), arguments.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        started,
        [(
            "read_file".to_string(),
            serde_json::json!({"path": "a.txt"})
        )]
    );
    assert!(events.iter().any(|e| matches!(e, ChatEvent::ToolFinished { ok: true, output, .. } if output == "1| hello\n2| world")));
    assert_eq!(ended(&events).len(), 2, "one turn per model step");

    // What the model saw.
    let bodies = chat_bodies(&srv);
    assert_eq!(bodies.len(), 2);
    assert_eq!(bodies[0]["tools"].as_array().unwrap().len(), 4);
    let system = bodies[0]["messages"][0]["content"].as_str().unwrap();
    assert!(system.contains("read-only agent"), "{system}");
    assert!(
        system.contains(&ws.path().display().to_string()),
        "{system}"
    );
    assert!(system.contains("Steps remaining: 20"), "{system}");
    let second = bodies[1]["messages"].as_array().unwrap();
    let roles: Vec<_> = second.iter().map(|m| m["role"].as_str().unwrap()).collect();
    assert_eq!(roles, ["system", "user", "assistant", "tool"]);
    assert_eq!(second[2]["tool_calls"][0]["function"]["name"], "read_file");
    assert_eq!(second[3]["tool_name"], "read_file");
    assert_eq!(second[3]["content"], "1| hello\n2| world");

    // What was saved.
    h.wait("saved", |_| {
        let t = texts(&h.read(id));
        t.last().is_some_and(|m| m.1 == "It says hello.")
    })
    .await;
    let s = h.read(id);
    assert_eq!(s.mode, crate::session::SessionMode::Agent);
    assert_eq!(s.workspace.as_deref(), Some(ws.path()));
    let kinds: Vec<_> = texts(&s).iter().map(|m| m.0.clone()).collect();
    assert_eq!(
        kinds,
        ["user", "assistant:Complete", "tool", "assistant:Complete"]
    );
    assert!(s.is_idle());
}

#[tokio::test]
async fn a_model_without_tool_support_cannot_run_an_agent() {
    let ws = workspace();
    let srv = fake(vec![chat_route(reply("never"))]).await; // show without "tools"
    let h = start(&srv).await;
    let id = h.create_agent(&ws, SessionLifetime::Persisted).await;
    h.send(id, "hello").await;
    h.wait("rejected", |e| {
        e.iter().any(|e| matches!(e, ChatEvent::SendRejected { reason: SendRejection::NoToolSupport(m), .. } if m == "llama3:8b"))
    })
    .await;
    assert!(chat_bodies(&srv).is_empty());
    assert!(h.read(id).messages.is_empty());
}

#[tokio::test]
async fn check_model_reports_whether_tools_are_supported() {
    let srv = fake_agent(vec![]).await;
    let h = start(&srv).await;
    h.cmd(ChatCommand::CheckModel("llama3:8b".into()));
    h.wait("capabilities", |e| {
        e.iter().any(|e| matches!(e, ChatEvent::ModelCapabilities { model, tools: true } if model == "llama3:8b"))
    })
    .await;
    let plain = fake(vec![]).await;
    let h2 = start(&plain).await;
    h2.cmd(ChatCommand::CheckModel("llama3:8b".into()));
    h2.wait("capabilities", |e| {
        e.iter()
            .any(|e| matches!(e, ChatEvent::ModelCapabilities { tools: false, .. }))
    })
    .await;
}

#[tokio::test]
async fn tool_failures_go_back_to_the_model_and_the_run_continues() {
    let ws = workspace();
    let srv = fake_agent(vec![
        chat_route(calls(
            "read_file",
            serde_json::json!({"path": "../etc/passwd"}),
        )),
        chat_route(calls("read_file", serde_json::json!({"path": ".env"}))),
        chat_route(reply("I could not read those.")),
    ])
    .await;
    let h = start(&srv).await;
    let id = h.create_agent(&ws, SessionLifetime::Persisted).await;
    h.send(id, "read secrets").await;
    h.wait_run_ended(1).await;
    let events = h.events();
    assert_eq!(run_ends(&events)[0].0, super::types::RunOutcome::Completed);
    let failures: Vec<String> = events
        .iter()
        .filter_map(|e| match e {
            ChatEvent::ToolFinished {
                ok: false, output, ..
            } => Some(output.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(failures.len(), 2);
    assert!(
        failures[0].starts_with("Error: ") && failures[0].contains("parent directory"),
        "{failures:?}"
    );
    assert!(failures[1].contains("protected"), "{failures:?}");
    let last = &chat_bodies(&srv)[2]["messages"];
    assert!(last
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m["role"] == "tool" && m["content"].as_str().unwrap().contains("protected")));
}

#[tokio::test]
async fn the_step_budget_ends_a_run_that_keeps_calling_tools() {
    let ws = workspace();
    let srv = fake_agent(vec![
        chat_route(calls("list_dir", serde_json::json!({}))),
        chat_route(calls("list_dir", serde_json::json!({}))),
        chat_route(reply("unreachable")),
    ])
    .await;
    let h = start(&srv).await;
    let id = h.create_agent(&ws, SessionLifetime::Persisted).await;
    h.cmd(ChatCommand::SetStepBudget(id, 2));
    h.send(id, "list forever").await;
    h.wait_run_ended(1).await;
    let (outcome, msg) = run_ends(&h.events())[0].clone();
    assert_eq!(outcome, super::types::RunOutcome::BudgetExhausted);
    assert!(msg.unwrap().contains("all 2 steps"));
    assert_eq!(chat_bodies(&srv).len(), 2, "no third request");
    let first_system = chat_bodies(&srv)[0]["messages"][0]["content"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(first_system.contains("Steps remaining: 2"));
    assert!(chat_bodies(&srv)[1]["messages"][0]["content"]
        .as_str()
        .unwrap()
        .contains("Steps remaining: 1"));
}

#[tokio::test]
async fn stop_ends_a_streaming_agent_run_once() {
    let ws = workspace();
    let (_keep, hold) = oneshot::channel::<()>();
    let srv = fake_agent(vec![chat_route(vec![
        Step::Write(line("thinking")),
        Step::WaitFor(hold),
    ])])
    .await;
    let h = start(&srv).await;
    let id = h.create_agent(&ws, SessionLifetime::Persisted).await;
    h.send(id, "go").await;
    h.wait("streaming", |_| h.deltas(id) == "thinking").await;
    h.cmd(ChatCommand::Stop(id));
    h.wait_run_ended(1).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let events = h.events();
    assert_eq!(
        run_ends(&events),
        vec![(super::types::RunOutcome::Cancelled, None)]
    );
    assert_eq!(ended(&events), vec![(TurnOutcome::Cancelled, None)]);
    h.wait("saved", |_| {
        texts(&h.read(id))
            .last()
            .is_some_and(|m| m.0 == "assistant:Cancelled")
    })
    .await;
}

#[tokio::test]
async fn steering_interrupts_the_reply_keeps_it_and_continues_the_same_run() {
    let ws = workspace();
    let (_keep, hold) = oneshot::channel::<()>();
    let srv = fake_agent(vec![
        chat_route(vec![Step::Write(line("partial")), Step::WaitFor(hold)]),
        chat_route(reply("ok, switching")),
    ])
    .await;
    let h = start(&srv).await;
    let id = h.create_agent(&ws, SessionLifetime::Persisted).await;
    h.send(id, "do the first thing").await;
    h.wait("streaming", |_| h.deltas(id) == "partial").await;
    h.send(id, "do the other thing instead").await;
    h.wait_run_ended(1).await;
    let events = h.events();
    assert_eq!(
        run_ends(&events),
        vec![(super::types::RunOutcome::Completed, None)],
        "one run, one end"
    );
    assert!(events.iter().any(|e| matches!(e, ChatEvent::SteeringQueued { text, .. } if text == "do the other thing instead")));
    assert_eq!(
        ended(&events).iter().map(|e| e.0).collect::<Vec<_>>(),
        [TurnOutcome::Interrupted, TurnOutcome::Completed]
    );
    h.wait("saved", |_| {
        texts(&h.read(id))
            .last()
            .is_some_and(|m| m.1 == "ok, switching")
    })
    .await;
    assert_eq!(
        texts(&h.read(id)),
        vec![
            ("user".into(), "do the first thing".into()),
            ("assistant:Interrupted".into(), "partial".into()),
            ("user".into(), "do the other thing instead".into()),
            ("assistant:Complete".into(), "ok, switching".into()),
        ]
    );
    let s = h.read(id);
    assert!(matches!(
        &s.messages[2].content,
        MessageContent::User {
            delivery: crate::session::UserDelivery::Steering,
            ..
        }
    ));
    let second = chat_bodies(&srv)[1]["messages"].as_array().unwrap().clone();
    let roles: Vec<_> = second.iter().map(|m| m["role"].as_str().unwrap()).collect();
    assert_eq!(roles, ["system", "user", "assistant", "user"]);
    assert_eq!(second[2]["content"], "partial");
    assert_eq!(second[3]["content"], "do the other thing instead");
}

#[tokio::test]
async fn the_workspace_tools_and_budget_cannot_change_under_a_running_agent() {
    let ws = workspace();
    let other = workspace();
    let (release, hold) = oneshot::channel();
    let srv = fake_agent(vec![chat_route(vec![
        Step::Write(line("x")),
        Step::WaitFor(hold),
        Step::Write(done()),
        Step::Close,
    ])])
    .await;
    let h = start(&srv).await;
    let id = h.create_agent(&ws, SessionLifetime::Persisted).await;
    h.send(id, "go").await;
    h.wait("streaming", |_| !h.deltas(id).is_empty()).await;
    h.cmd(ChatCommand::SetWorkspace(id, other.path().to_path_buf()));
    h.cmd(ChatCommand::SetToolEnabled(id, "search_text".into(), false));
    h.cmd(ChatCommand::SetStepBudget(id, 3));
    h.wait("three refusals", |e| {
        e.iter()
            .filter(|e| matches!(e, ChatEvent::SessionError { message, .. } if message.contains("Stop the current run")))
            .count()
            >= 3
    })
    .await;
    release.send(()).unwrap();
    h.wait_run_ended(1).await;
    // Idle again: the same changes now apply.
    h.cmd(ChatCommand::SetWorkspace(id, other.path().to_path_buf()));
    h.cmd(ChatCommand::SetToolEnabled(id, "search_text".into(), false));
    h.wait("applied", |_| {
        let s = h.read(id);
        s.workspace.as_deref() == Some(other.path())
            && s.effective_tools.tools.get("search_text") == Some(&false)
    })
    .await;
}

#[tokio::test]
async fn switched_off_tools_are_not_offered_to_the_model() {
    let ws = workspace();
    let srv = fake_agent(vec![chat_route(reply("done"))]).await;
    let h = start(&srv).await;
    let id = h.create_agent(&ws, SessionLifetime::Persisted).await;
    h.cmd(ChatCommand::SetToolEnabled(id, "search_text".into(), false));
    h.cmd(ChatCommand::SetCapability(
        id,
        "files.read".into(),
        crate::session::Permission::Auto,
    ));
    h.send(id, "hi").await;
    h.wait_run_ended(1).await;
    let names: Vec<_> = chat_bodies(&srv)[0]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["function"]["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(names, ["list_dir", "read_file", "find_files"]);
}

#[tokio::test]
async fn an_agent_needs_a_real_workspace_folder() {
    let srv = fake_agent(vec![]).await;
    let h = start(&srv).await;
    let nowhere = TempDir::new().unwrap().path().join("missing");
    h.cmd(ChatCommand::CreateAgent {
        lifetime: SessionLifetime::Persisted,
        model: "m".into(),
        workspace: nowhere,
    });
    h.cmd(ChatCommand::CreateAgent {
        lifetime: SessionLifetime::Persisted,
        model: "m".into(),
        workspace: std::path::PathBuf::from("relative/dir"),
    });
    h.wait("two refusals", |e| {
        e.iter()
            .filter(|e| matches!(e, ChatEvent::SessionError { session: None, .. }))
            .count()
            >= 2
    })
    .await;
    assert!(h.opened().is_empty());
}

#[tokio::test]
async fn an_incognito_agent_run_leaves_the_data_dir_untouched() {
    let ws = workspace();
    let srv = fake_agent(vec![
        chat_route(calls("read_file", serde_json::json!({"path": "a.txt"}))),
        chat_route(reply("secret findings")),
    ])
    .await;
    let h = start(&srv).await;
    let before = dir_snapshot(h.dir.path());
    let id = h.create_agent(&ws, SessionLifetime::Incognito).await;
    h.send(id, "secret task").await;
    h.wait_run_ended(1).await;
    h.cmd(ChatCommand::ClearIncognito);
    h.handle.shutdown().await;
    let after = dir_snapshot(h.dir.path());
    assert_eq!(after, before);
    for (name, bytes) in &after {
        assert!(!String::from_utf8_lossy(bytes).contains("secret"), "{name}");
    }
}

#[tokio::test]
async fn deleting_a_session_in_the_middle_of_a_tool_run_leaves_nothing_behind() {
    let ws = workspace();
    let (_keep, hold) = oneshot::channel::<()>();
    let srv = fake_agent(vec![chat_route(vec![
        Step::Write(line("x")),
        Step::WaitFor(hold),
    ])])
    .await;
    let h = start(&srv).await;
    let id = h.create_agent(&ws, SessionLifetime::Persisted).await;
    h.send(id, "go").await;
    h.wait("streaming", |_| !h.deltas(id).is_empty()).await;
    h.cmd(ChatCommand::Delete(id));
    h.wait("gone", |e| {
        e.iter()
            .rev()
            .take(4)
            .any(|e| matches!(e, ChatEvent::Rows(r) if r.iter().all(|x| x.id != id)))
    })
    .await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(!h.file(id).exists());
    assert!(run_ends(&h.events()).is_empty());
}

#[tokio::test]
async fn changing_an_idle_agents_settings_reports_them() {
    let ws = workspace();
    let other = workspace();
    let srv = fake_agent(vec![]).await;
    let h = start(&srv).await;
    let id = h.create_agent(&ws, SessionLifetime::Persisted).await;
    h.cmd(ChatCommand::SetToolEnabled(id, "find_files".into(), false));
    h.cmd(ChatCommand::SetStepBudget(id, 7));
    h.cmd(ChatCommand::SetWorkspace(id, other.path().to_path_buf()));
    h.wait("settings", |e| {
        e.iter().any(|e| {
            matches!(e, ChatEvent::AgentSettings { step_budget: 7, workspace: Some(w), tools, .. }
            if w == other.path() && tools.tools.get("find_files") == Some(&false))
        })
    })
    .await;
}

#[tokio::test]
async fn regenerate_keeps_the_old_reply_as_a_version_and_resends_the_question() {
    let srv = fake(vec![chat_route(reply("one")), chat_route(reply("two"))]).await;
    let h = start(&srv).await;
    let id = h.create(SessionLifetime::Persisted).await;
    h.send(id, "ask").await;
    h.wait_ended(1).await;
    h.cmd(ChatCommand::Regenerate(id));
    h.wait_ended(2).await;
    h.wait("saved", |_| {
        let s = h.read(id);
        texts(&s)
            == [
                ("user".into(), "ask".into()),
                ("assistant:Complete".into(), "two".into()),
            ]
            && s.forks.len() == 1
    })
    .await;
    let s = h.read(id);
    let new_reply = s.messages.last().unwrap().id;
    assert_eq!(s.version_of(new_reply), Some((2, 2)));
    assert_eq!(texts(&s_with_version(s, 0)).last().unwrap().1, "one");
    let second = &chat_bodies(&srv)[1]["messages"];
    assert_eq!(second.as_array().unwrap().len(), 2, "{second}");
    assert_eq!(second[1]["content"], "ask");
}

/// The saved chat with version `to` of its first fork showing.
fn s_with_version(mut s: Session, to: usize) -> Session {
    let after = s.forks[0].after;
    assert!(s.switch_version(after, to));
    s
}

#[tokio::test]
async fn regenerate_sends_the_window_a_fresh_copy_of_the_chat() {
    let srv = fake(vec![chat_route(reply("one")), chat_route(reply("two"))]).await;
    let h = start(&srv).await;
    let id = h.create(SessionLifetime::Persisted).await;
    h.send(id, "ask").await;
    h.wait_ended(1).await;
    h.cmd(ChatCommand::Regenerate(id));
    h.wait_ended(2).await;
    let evs = h.events();
    let opened = evs
        .iter()
        .rposition(
            |e| matches!(e, ChatEvent::Opened(s) if s.forks.len() == 1 && s.messages.len() == 2),
        )
        .expect("Opened with the new version");
    let started = evs
        .iter()
        .rposition(|e| matches!(e, ChatEvent::TurnStarted { .. }))
        .unwrap();
    assert!(
        opened < started,
        "snapshot {opened} after turn start {started}"
    );
}

#[tokio::test]
async fn editing_a_message_makes_a_new_version_and_asks_again_from_there() {
    let srv = fake(vec![chat_route(reply("one")), chat_route(reply("two"))]).await;
    let h = start(&srv).await;
    let id = h.create(SessionLifetime::Persisted).await;
    h.send(id, "ask").await;
    h.wait_ended(1).await;
    let first = h.read(id).messages[0].id;
    h.cmd(ChatCommand::Edit {
        session: id,
        message: first,
        text: "better ask".into(),
    });
    h.wait_ended(2).await;
    h.wait("saved", |_| {
        let s = h.read(id);
        texts(&s).len() == 2 && texts(&s)[1].1 == "two" && s.forks.len() == 1
    })
    .await;
    let s = h.read(id);
    assert_eq!(texts(&s)[0].1, "better ask");
    assert_eq!(s.version_of(s.messages[0].id), Some((2, 2)));
    let old = s_with_version(s, 0);
    assert_eq!(texts(&old)[0].1, "ask");
    assert_eq!(texts(&old)[1].1, "one");
    let second = &chat_bodies(&srv)[1]["messages"];
    assert_eq!(second.as_array().unwrap().len(), 2, "{second}");
    assert_eq!(second[1]["content"], "better ask");
}

#[tokio::test]
async fn an_edit_of_anything_but_your_own_message_is_refused() {
    let srv = fake(vec![chat_route(reply("one"))]).await;
    let h = start(&srv).await;
    let id = h.create(SessionLifetime::Persisted).await;
    h.send(id, "ask").await;
    h.wait_ended(1).await;
    let reply_id = h.read(id).messages[1].id;
    for (message, text) in [
        (reply_id, "x"),
        (crate::session::MessageId::new(), "x"),
        (h.read(id).messages[0].id, "   "),
    ] {
        h.cmd(ChatCommand::Edit {
            session: id,
            message,
            text: text.into(),
        });
    }
    h.wait("refused", |evs| {
        evs.iter()
            .filter(|e| {
                matches!(
                    e,
                    ChatEvent::SessionError { .. } | ChatEvent::SendRejected { .. }
                )
            })
            .count()
            >= 3
    })
    .await;
    assert_eq!(chat_bodies(&srv).len(), 1, "no new request");
    assert!(h.read(id).forks.is_empty());
}

#[tokio::test]
async fn switching_versions_changes_what_is_shown_and_saved() {
    let srv = fake(vec![chat_route(reply("one")), chat_route(reply("two"))]).await;
    let h = start(&srv).await;
    let id = h.create(SessionLifetime::Persisted).await;
    h.send(id, "ask").await;
    h.wait_ended(1).await;
    h.cmd(ChatCommand::Regenerate(id));
    h.wait_ended(2).await;
    h.wait("saved", |_| h.read(id).forks.len() == 1).await;
    let user = h.read(id).messages[0].id;
    h.cmd(ChatCommand::SwitchVersion {
        session: id,
        after: Some(user),
        to: 0,
    });
    let shows_one = |evs: &[ChatEvent]| {
        evs.iter().rev().find_map(|e| match e {
            ChatEvent::Opened(s) => Some(texts(s)[1].1 == "one"),
            _ => None,
        }) == Some(true)
    };
    h.wait("window told", shows_one).await;
    h.wait("saved", |_| texts(&h.read(id))[1].1 == "one").await;
}

#[tokio::test]
async fn regenerate_does_nothing_when_the_last_reply_did_not_complete() {
    let srv = fake(vec![chat_route(vec![
        Step::Write(line("half")),
        Step::Close,
    ])])
    .await;
    let h = start(&srv).await;
    let id = h.create(SessionLifetime::Persisted).await;
    h.send(id, "ask").await;
    h.wait_ended(1).await;
    h.cmd(ChatCommand::Regenerate(id));
    h.wait("refused", |evs| {
        evs.iter()
            .any(|e| matches!(e, ChatEvent::SessionError { .. }))
    })
    .await;
    assert_eq!(chat_bodies(&srv).len(), 1, "no second request");
    assert_eq!(texts(&h.read(id)).len(), 2, "the failed reply is kept");
}

#[tokio::test]
async fn a_finished_reply_keeps_the_numbers_ollama_reported() {
    let done_with_stats = format!(
        "{}\n",
        serde_json::json!({
            "message": {"role": "assistant", "content": ""},
            "done": true, "done_reason": "stop",
            "prompt_eval_count": 12, "eval_count": 40,
            "total_duration": 2_500_000_000u64,
            "load_duration": 300_000_000u64,
            "eval_duration": 2_000_000_000u64
        })
    )
    .into_bytes();
    let srv = fake(vec![chat_route(vec![
        Step::Write(line("hi")),
        Step::Write(done_with_stats),
        Step::Close,
    ])])
    .await;
    let h = start(&srv).await;
    let id = h.create(SessionLifetime::Persisted).await;
    h.send(id, "ask").await;
    h.wait_ended(1).await;
    h.wait("saved", |_| {
        h.read(id).messages.iter().any(|m| {
            matches!(&m.content, MessageContent::Assistant { stats: Some(s), .. }
                if s.prompt_tokens == Some(12) && s.reply_tokens == Some(40)
                && s.total_ms == Some(2_500) && s.load_ms == Some(300) && s.eval_ms == Some(2_000))
        })
    })
    .await;
}

async fn chat_with_one_exchange(h: &H) -> (SessionId, MessageId) {
    let id = h.create(SessionLifetime::Persisted).await;
    h.send(id, "ask").await;
    h.wait_ended(1).await;
    h.wait("saved", |_| {
        h.try_read(id).is_some_and(|s| s.messages.len() == 2)
    })
    .await;
    (id, h.read(id).messages[1].id)
}

#[tokio::test]
async fn branching_creates_a_new_chat_with_the_history_and_opens_it() {
    let srv = fake(vec![chat_route(reply("one"))]).await;
    let h = start(&srv).await;
    let (id, at) = chat_with_one_exchange(&h).await;
    h.cmd(ChatCommand::Branch {
        session: id,
        message: at,
        model: Some("other:7b".into()),
    });
    h.wait("opened", |evs| {
        evs.iter()
            .any(|e| matches!(e, ChatEvent::Opened(s) if s.id != id && s.model == "other:7b"))
    })
    .await;
    let new_id = h
        .events()
        .iter()
        .rev()
        .find_map(|e| match e {
            ChatEvent::Opened(s) if s.id != id => Some(s.id),
            _ => None,
        })
        .unwrap();
    h.wait("saved", |_| h.read(new_id).messages.len() == 2)
        .await;
    let b = h.read(new_id);
    assert_eq!(
        b.parent.map(|p| (p.session_id, p.message_id)),
        Some((id, at))
    );
    assert_eq!(b.lineage.len(), 1);
    assert_eq!(h.read(id).messages.len(), 2, "the original is untouched");
}

#[tokio::test]
async fn branching_from_an_unfinished_reply_is_refused() {
    let srv = fake(vec![chat_route(reply("one"))]).await;
    let h = start(&srv).await;
    let (id, _) = chat_with_one_exchange(&h).await;
    let user = h.read(id).messages[0].id;
    h.cmd(ChatCommand::Branch {
        session: id,
        message: user,
        model: None,
    });
    h.wait("refused", |evs| {
        evs.iter()
            .any(|e| matches!(e, ChatEvent::SessionError { .. }))
    })
    .await;
    assert!(!h
        .events()
        .iter()
        .any(|e| matches!(e, ChatEvent::Opened(s) if s.id != id)));
}

async fn branch_and_open(h: &H, id: SessionId, at: MessageId) -> SessionId {
    h.cmd(ChatCommand::Branch {
        session: id,
        message: at,
        model: None,
    });
    h.wait("opened", |evs| {
        evs.iter()
            .any(|e| matches!(e, ChatEvent::Opened(s) if s.id != id))
    })
    .await;
    let new_id = h
        .events()
        .iter()
        .rev()
        .find_map(|e| match e {
            ChatEvent::Opened(s) if s.id != id => Some(s.id),
            _ => None,
        })
        .unwrap();
    h.wait("saved", |_| h.read(new_id).messages.len() == 2)
        .await;
    new_id
}

fn last_branch_state(h: &H, session: SessionId) -> Option<BranchState> {
    h.events().iter().rev().find_map(|e| match e {
        ChatEvent::BranchChecked { session: s, state } if *s == session => Some(*state),
        _ => None,
    })
}

#[tokio::test]
async fn a_branch_reports_whether_its_original_changed_or_is_gone() {
    let srv = fake(vec![chat_route(reply("one")), chat_route(reply("two"))]).await;
    let h = start(&srv).await;
    let (id, at) = chat_with_one_exchange(&h).await;
    let b = branch_and_open(&h, id, at).await;

    h.cmd(ChatCommand::CheckBranch(b));
    h.wait("unchanged", |_| {
        last_branch_state(&h, b) == Some(BranchState::Unchanged)
    })
    .await;

    // Regenerating the branch point's reply changes the original's history.
    h.cmd(ChatCommand::Regenerate(id));
    h.wait_ended(2).await;
    h.wait("saved", |_| h.read(id).forks.len() == 1).await;
    h.cmd(ChatCommand::CheckBranch(b));
    h.wait("changed", |_| {
        last_branch_state(&h, b) == Some(BranchState::Changed)
    })
    .await;

    // Switching back makes it match again.
    let user = h.read(id).messages[0].id;
    h.cmd(ChatCommand::SwitchVersion {
        session: id,
        after: Some(user),
        to: 0,
    });
    h.wait("saved back", |_| h.read(id).messages[1].id == at)
        .await;
    h.cmd(ChatCommand::CheckBranch(b));
    h.wait("unchanged again", |evs| {
        evs.iter()
            .filter(|e| {
                matches!(
                    e,
                    ChatEvent::BranchChecked {
                        state: BranchState::Unchanged,
                        ..
                    }
                )
            })
            .count()
            >= 2
    })
    .await;

    h.cmd(ChatCommand::Delete(id));
    h.wait("deleted", |_| {
        h.cmd(ChatCommand::CheckBranch(b));
        last_branch_state(&h, b) == Some(BranchState::Deleted)
    })
    .await;
}

fn btw_done(h: &H, session: SessionId) -> Option<crate::session::BtwEntry> {
    h.events().iter().rev().find_map(|e| match e {
        ChatEvent::BtwDone { session: s, entry } if *s == session => Some(entry.clone()),
        _ => None,
    })
}

#[tokio::test]
async fn a_side_question_is_answered_from_the_history_and_stays_out_of_the_chat() {
    let srv = fake(vec![chat_route(reply("one")), chat_route(reply("side"))]).await;
    let h = start(&srv).await;
    let (id, at) = chat_with_one_exchange(&h).await;
    h.cmd(ChatCommand::Btw {
        session: id,
        message: at,
        question: "why?".into(),
    });
    h.wait("answered", |_| btw_done(&h, id).is_some()).await;
    let entry = btw_done(&h, id).unwrap();
    assert_eq!(
        (entry.question.as_str(), entry.answer.as_str()),
        ("why?", "side")
    );
    let body = &chat_bodies(&srv)[1]["messages"];
    let roles: Vec<&str> = body
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["role"].as_str().unwrap())
        .collect();
    assert_eq!(roles, ["system", "user", "assistant", "user"]);
    assert_eq!(body[3]["content"], "why?");
    h.wait("saved", |_| {
        h.try_read(id).is_some_and(|s| s.btw.len() == 1)
    })
    .await;
    let s = h.read(id);
    assert_eq!(s.messages.len(), 2, "the transcript is untouched");
}

#[tokio::test]
async fn a_side_question_that_fails_says_so_and_is_not_kept() {
    let srv = fake(vec![
        chat_route(reply("one")),
        chat_route(vec![Step::Close]),
    ])
    .await;
    let h = start(&srv).await;
    let (id, at) = chat_with_one_exchange(&h).await;
    h.cmd(ChatCommand::Btw {
        session: id,
        message: at,
        question: "why?".into(),
    });
    h.wait("failed", |evs| {
        evs.iter()
            .any(|e| matches!(e, ChatEvent::BtwFailed { session, .. } if *session == id))
    })
    .await;
    assert!(h.read(id).btw.is_empty());
}

#[tokio::test]
async fn adding_a_side_question_appends_it_and_deleting_forgets_it() {
    let srv = fake(vec![chat_route(reply("one")), chat_route(reply("side"))]).await;
    let h = start(&srv).await;
    let (id, at) = chat_with_one_exchange(&h).await;
    h.cmd(ChatCommand::Btw {
        session: id,
        message: at,
        question: "why?".into(),
    });
    h.wait("answered", |_| btw_done(&h, id).is_some()).await;
    let entry = btw_done(&h, id).unwrap();
    h.cmd(ChatCommand::BtwAdd {
        session: id,
        id: entry.id,
    });
    h.wait("added", |_| {
        h.try_read(id).is_some_and(|s| s.messages.len() == 4)
    })
    .await;
    let s = h.read(id);
    assert_eq!(texts(&s)[3], ("assistant:Complete".into(), "side".into()));
    assert!(s.btw[0].added);
    h.cmd(ChatCommand::BtwDelete {
        session: id,
        id: entry.id,
    });
    h.wait("deleted", |_| {
        h.try_read(id).is_some_and(|s| s.btw.is_empty())
    })
    .await;
    assert_eq!(
        h.read(id).messages.len(),
        4,
        "deleting does not undo the add"
    );
}
