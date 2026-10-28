# Roost Stage 2 (Streaming Chat, Persistence, History Rail) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A user can create a saved or incognito chat, stream a reply from an installed Ollama model, restart Roost and continue, with a history rail that lists, pins, renames and deletes sessions while runs continue in the background.

**Architecture:** A single session-manager task in `roost-core::chat` owns sessions, the incognito map and the run rule; a writer thread in `roost-core::store` owns all disk I/O. `roost-gui` sends `ChatCommand`s through the existing backend channel and folds `ChatEvent`s into a view-model; it does no HTTP or file work.

**Tech Stack:** Rust 2021 (MSRV 1.95), tokio, reqwest 0.13 (rustls, `stream` already enabled), serde, egui/eframe 0.37, wiremock and a raw-TCP test server, tempfile.

**Spec:** `docs/superpowers/specs/2026-10-27-roost-chat-design.md` (parent: `2026-10-27-roost-workbench-design.md`)

## Global Constraints

- Test first; watch each test fail before implementing. No `unwrap`/`expect` on I/O or network in non-test code.
- Tests never touch the real Ollama (port 11434), the real keychain or the real config. Use wiremock, the TCP test server and tempdirs.
- No OpenSSL. rustls only.
- Persisted format stays `SESSION_SCHEMA_VERSION = 1`; any new field is additive with a serde default.
- Window: explicit `options.num_ctx` on every chat request = `session.model_options.num_ctx`, else `min(show context, 16_384)`, else `8_192`.
- Token estimate `ceil(UTF-8 bytes / 3) + 8` per message; reserve 30% of the window for output.
- First-event timeout 5 minutes; `chat_stall` 120 seconds between events; flush interval `FLUSH_INTERVAL = 2s`; delta coalescing window about 30 ms; shutdown waits at most 3 s.
- Incognito data never reaches the store, index, logs, recovery files or eframe persistence.
- Every task ends with `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` green. No push, merge, tag or release without being asked. No email addresses in files.
- All colours come from theme tokens; Chat layout must pass at 900x640 in all four families, light and dark.

## Review Focus

1. A stream that stalls mid-reply or closes without `done:true` ends Failed once, keeps the partial text, and offers Retry (Task 4, Task 8).
2. A multi-byte UTF-8 character split across network chunks decodes intact (Task 4).
3. Stop racing with completion produces exactly one terminal event and no text after it (Task 8).
4. Delete or quit during a run leaves no resurrected file and no lost partial reply (Task 9).
5. A corrupt, newer-schema or unreadable session file never hides other sessions and is never damaged (Task 5).

---

### Task 1: Session API for chat

**Files:**
- Modify: `crates/roost-core/src/session.rs`

**Interfaces:**
- Produces:
  - `MessageContent::Assistant` gains `thinking: Option<String>` (`#[serde(default, skip_serializing_if = "Option::is_none")]`).
  - `Session::rename(&mut self, title: &str) -> Result<(), SessionError>` (no `now_ms`, no `updated_at` change).
  - `Session::set_pinned(&mut self, pinned: bool)` (no `updated_at` change).
  - `Session::push_user(&mut self, text: &str, now_ms: u64) -> MessageId` (sets `updated_at_ms`).
  - `Session::begin_assistant(&mut self, run: RunId, turn: TurnId, now_ms: u64) -> MessageId` (status `Streaming`, `run_state = Running{run,turn}`).
  - `Session::append_assistant(&mut self, id: MessageId, content: &str, thinking: &str)`.
  - `Session::finish_assistant(&mut self, id: MessageId, status: AssistantStatus, error: Option<String>, now_ms: u64)` (sets `run_state = Idle`, `updated_at_ms`).

- [ ] **Step 1: Write failing tests** in `session.rs` tests: `rename_does_not_change_updated_at`; `set_pinned_does_not_change_updated_at`; `push_user_appends_normal_user_message_and_bumps_updated_at`; `begin_append_finish_builds_one_complete_assistant_message` (text and thinking accumulate, status `Complete`, `run_state` Idle); `old_assistant_json_without_thinking_still_loads`; `thinking_roundtrips`. Update the existing test at the `updated_at_ms == T0 + 50` assertion to match the new rename rule.
- [ ] **Step 2:** `cargo test -p roost-core session` — Expected: compile errors / FAIL.
- [ ] **Step 3:** Implement the signatures above; fix every `Assistant { .. }` construction and match in the workspace (`cargo build --workspace`).
- [ ] **Step 4:** `cargo test --workspace` — Expected: PASS.
- [ ] **Step 5:** Commit `feat(core): session helpers for chat; thinking field; rename leaves updated_at alone`.

### Task 2: Context window and trimming

**Files:**
- Create: `crates/roost-core/src/chat/mod.rs` (declares `pub mod context;`), `crates/roost-core/src/chat/context.rs`
- Modify: `crates/roost-core/src/lib.rs` (add `pub mod chat;`)

**Interfaces:**
- Consumes: `session::{Message, MessageContent, AssistantStatus}`.
- Produces (in `chat::context`):
  - `pub const WINDOW_CAP: u32 = 16_384; pub const FALLBACK_WINDOW: u32 = 8_192; pub const OMITTED_MARKER: &str = "Earlier messages omitted for context";`
  - `pub fn window(session_num_ctx: Option<u32>, show_context: Option<u64>) -> u32`
  - `pub struct WireMessage { pub role: String, pub content: String }` (re-exported from `ollama` in Task 4; define here first as the single definition and have `ollama` use it).
  - `pub struct Trimmed { pub messages: Vec<WireMessage>, pub omitted: usize }`
  - `pub enum ContextError { TooLarge { needed: usize, budget: usize } }` (implements `Display` with a readable message)
  - `pub fn build_request(system: &str, transcript: &[Message], window: u32) -> Result<Trimmed, ContextError>`

Rules: budget = `window * 70 / 100`. A *group* is a user message plus the `Complete` assistant message after it. An exchange whose reply is not `Complete` is dropped whole when a later user message exists; the newest user message is always kept (alone, if its reply failed). Never send thinking. Keep the newest groups that fit; when older groups were dropped for size, put one `system` message with `OMITTED_MARKER` right after the system prompt and count dropped messages in `omitted`. Dropping failed exchanges does not count toward `omitted`.

- [ ] **Step 1: Write failing tests**: `window_prefers_session_override`; `window_caps_show_context_at_16384`; `window_falls_back_to_8192`; `exact_fit_keeps_everything`; `one_byte_over_drops_the_oldest_group_and_adds_marker` (asserts `omitted == 2`); `never_starts_with_an_assistant_message`; `failed_exchange_followed_by_new_user_message_is_dropped_whole_without_counting_as_omitted`; `newest_user_message_is_kept_when_its_reply_failed`; `thinking_is_never_sent`; `system_plus_newest_too_large_is_an_error`.
- [ ] **Step 2:** `cargo test -p roost-core chat::context` — Expected: FAIL (not defined).
- [ ] **Step 3:** Implement per the rules above; token estimate is a private `fn tokens(text: &str) -> usize`.
- [ ] **Step 4:** Run the same tests — Expected: PASS.
- [ ] **Step 5:** Commit `feat(core): chat context window and trimming`.

### Task 3: Raw TCP test server and `Error::ModelNotFound`

**Files:**
- Create: `crates/roost-core/src/test_server.rs` (`#[cfg(test)]`, declared in `lib.rs`)
- Modify: `crates/roost-core/src/error.rs`, any exhaustive `match` on `Error` the compiler flags

**Interfaces:**
- Produces:
  - `pub enum Step { Write(Vec<u8>), Pause(Duration), WaitFor(tokio::sync::oneshot::Receiver<()>), Close }`
  - `pub struct TcpScript` with `pub async fn start(status_line: &str, headers: &str, steps: Vec<Step>) -> TcpScript`, `pub fn base_url(&self) -> String`, `pub async fn request_body(&self) -> String` (the body the client sent, for asserting `num_ctx`).
  - `Error::ModelNotFound(String)` with message `"model not found: {0}"`.

- [ ] **Step 1: Write failing tests**: `script_writes_bytes_split_exactly_where_told` (client reads two chunks in order); `script_waits_for_a_signal_before_continuing`; `script_records_the_request_body`; `model_not_found_displays_the_name`.
- [ ] **Step 2:** `cargo test -p roost-core test_server error` — Expected: FAIL.
- [ ] **Step 3:** Implement with `tokio::net::TcpListener` on `127.0.0.1:0`, one connection, response written as HTTP/1.1 with `Connection: close` and chunked transfer so scripted splits are preserved.
- [ ] **Step 4:** Run — Expected: PASS.
- [ ] **Step 5:** Commit `test(core): raw TCP script server; Error::ModelNotFound`.

### Task 4: `OllamaClient::chat_stream`

**Files:**
- Modify: `crates/roost-core/src/ollama.rs`

**Interfaces:**
- Consumes: `chat::context::WireMessage`, `test_server::{TcpScript, Step}`, existing `check`/`error_message`.
- Produces:
  - `pub struct ChatRequestBody { pub model: String, pub messages: Vec<WireMessage>, pub num_ctx: u32, pub temperature: Option<f32>, pub seed: Option<i64>, pub think: Option<bool> }`
  - `pub enum StreamEvent { Content(String), Thinking(String), Done { prompt_eval_count: Option<u64>, eval_count: Option<u64>, done_reason: Option<String> } }`
  - `pub struct ChatStream` with `pub async fn next(&mut self) -> Result<Option<StreamEvent>>`: yields events, `Done` last, then `Ok(None)`.
  - `OllamaClient::chat_stream(&self, body: &ChatRequestBody) -> Result<ChatStream>`; `with_first_event_timeout(self, Duration) -> Self`; `with_stall_timeout` now also sets `chat_stall` (default 120 s); first-event default 300 s.
  - A `check` variant that keeps the status so a 404 whose JSON body says the model is missing becomes `Error::ModelNotFound(name)`; other statuses behave as today.

Decoder buffers bytes and splits on `\n`. Request sets `stream: true`, `options.num_ctx` always, `temperature`/`seed` and top-level `think` only when `Some`.

- [ ] **Step 1: Write failing tests** (all via `TcpScript` unless noted): `content_and_thinking_deltas_arrive_in_order`; `a_line_split_across_chunks_is_reassembled`; `a_multibyte_character_split_across_chunks_decodes_intact`; `several_lines_in_one_chunk_yield_several_events`; `the_request_body_has_stream_true_and_num_ctx` (asserts via `request_body`); `a_404_json_error_becomes_model_not_found`; `a_500_json_error_becomes_ollama_error`; `an_in_stream_error_line_ends_with_ollama_error`; `a_malformed_line_is_a_parse_error`; `closing_without_done_is_a_network_error_connection_ended_early`; `a_stall_after_the_first_event_times_out` (uses `with_stall_timeout(50ms)`); `no_first_event_times_out` (uses `with_first_event_timeout(50ms)`); `done_carries_prompt_eval_count`.
- [ ] **Step 2:** `cargo test -p roost-core ollama::` — Expected: new tests FAIL.
- [ ] **Step 3:** Implement. Loop on `resp.chunk()` wrapped in `tokio::time::timeout`, as `create` does.
- [ ] **Step 4:** Run — Expected: PASS, existing ollama tests still green.
- [ ] **Step 5:** Commit `feat(core): streaming /api/chat client`.

### Task 5: Session store and index

**Files:**
- Create: `crates/roost-core/src/store.rs`
- Modify: `crates/roost-core/src/lib.rs`

**Interfaces:**
- Consumes: `session::{Session, SessionDocument, SessionId, SessionError}`, `persistence::atomic_write`.
- Produces:
  - `pub enum RowStatus { Ok, Quarantined, NewerSchema, Unreadable }`
  - `pub struct IndexRow { pub id: SessionId, pub title: String, pub mode: SessionMode, pub lifetime: SessionLifetime, pub pinned: bool, pub model: String, pub parent: Option<BranchOrigin>, pub created_at_ms: u64, pub updated_at_ms: u64, pub status: RowStatus, pub file_mtime_ms: u64, pub file_len: u64 }` (serde)
  - `pub enum StoreWarning { IndexRebuilt, Quarantined(SessionId), NewerSchema(SessionId), Unreadable(SessionId) }`
  - `pub enum StoreError { Locked, Io(String) }`
  - `pub struct Store`; `Store::open(dir: &Path) -> Result<(Store, Vec<StoreWarning>), StoreError>` (creates the dir, takes the `.lock` with `File::try_lock`, reconciles); `rows(&self) -> Vec<IndexRow>`; `load(&mut self, id: SessionId) -> Result<Session, LoadError>`; `save(&mut self, s: &Session) -> Result<(), StoreError>`; `delete(&mut self, id: SessionId) -> Result<(), StoreError>`.
  - `pub enum LoadError { Missing, Quarantined, NewerSchema, Unreadable(String) }`

Reconcile on open: add files missing from the index, drop rows without files, refresh rows whose recorded mtime or length differ (mtime and length are the staleness signal). A missing, unreadable or wrong-version index is rebuilt (`IndexRebuilt`). Parse or validation failure → rename to `<id>.json.corrupt-<unix-ms>`; scan lists existing `*.corrupt-*` files as `Quarantined` rows. Schema newer than current → untouched, `NewerSchema`. Transient I/O error → `Unreadable`, no rename. `save` writes the session file via `atomic_write`, then the index only if the row changed. Normalize with `normalize_after_crash` on first load at startup, and save if it changed.

- [ ] **Step 1: Write failing tests**: `save_then_load_roundtrips`; `saving_one_session_does_not_touch_other_files` (compare mtimes/bytes); `missing_index_is_rebuilt_from_files`; `unsupported_index_version_is_rebuilt`; `stale_index_row_is_refreshed_by_mtime`; `row_without_a_file_is_dropped`; `a_corrupt_file_is_quarantined_and_others_still_list`; `quarantined_rows_survive_an_index_rebuild`; `a_newer_schema_file_is_left_untouched_and_marked`; `a_transient_read_error_does_not_rename` (make the file unreadable with permissions, Unix only, `#[cfg(unix)]`); `pin_survives_an_index_rebuild`; `a_crash_left_streaming_session_is_normalized_on_load`; `a_second_open_on_the_same_dir_is_locked`; `delete_removes_file_and_row`.
- [ ] **Step 2:** `cargo test -p roost-core store` — Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run — Expected: PASS.
- [ ] **Step 5:** Commit `feat(core): per-session store with derived index`.

### Task 6: Store writer

**Files:**
- Modify: `crates/roost-core/src/store.rs` (add `Writer`) 

**Interfaces:**
- Consumes: `Store`.
- Produces:
  - `pub struct Writer` (cloneable handle); `Writer::spawn(store: Store) -> Writer` (a dedicated thread owning the store).
  - `Writer::save(&self, s: Session)` (fire-and-forget), `Writer::delete(&self, id: SessionId)` (adds a tombstone; later `save` for that id is dropped), `async fn snapshot(&self) -> Vec<IndexRow>`, `async fn load(&self, id: SessionId) -> Result<Session, LoadError>`, `async fn flush(&self)` (returns when queued jobs are done), `fn take_warnings(&self) -> Vec<StoreWarning>`.

- [ ] **Step 1: Write failing tests**: `saves_are_applied_in_order_and_flush_waits`; `a_save_after_delete_does_not_recreate_the_file`; `snapshot_reflects_queued_saves`; `dropping_all_handles_stops_the_thread_after_draining`.
- [ ] **Step 2:** `cargo test -p roost-core store::writer` — Expected: FAIL.
- [ ] **Step 3:** Implement with a `std::sync::mpsc` channel and `tokio::sync::oneshot` replies.
- [ ] **Step 4:** Run — Expected: PASS.
- [ ] **Step 5:** Commit `feat(core): store writer thread with tombstones`.

### Task 7: Manager types and session lifecycle

**Files:**
- Create: `crates/roost-core/src/chat/types.rs`, `crates/roost-core/src/chat/manager.rs`
- Modify: `crates/roost-core/src/chat/mod.rs`

**Interfaces:**
- Consumes: `store::{Writer, IndexRow}`, `session::*`.
- Produces in `chat::types`:
  - `pub enum ChatCommand { Create { lifetime: SessionLifetime, model: String }, Open(SessionId), Close(SessionId), ClearIncognito, Rename(SessionId, String), SetPinned(SessionId, bool), Delete(SessionId), SetModel(SessionId, String), Send { session: SessionId, text: String }, Stop(SessionId), Retry(SessionId), ListSessions, InstalledChanged, Shutdown }`
  - `pub enum SendRejection { Busy, TooLarge(String), UnknownSession }`
  - `pub enum ChatEvent { Rows(Vec<IndexRow>), Opened(Session), Created(SessionId), TurnStarted { session, run, turn }, Loading { session }, WaitingForModel { session }, Delta { session, turn, content: String, thinking: String }, ContextTrimmed { session, omitted: usize }, TurnEnded { session, turn, outcome: TurnOutcome, message: Option<String> }, Usage { session, turn, prompt_eval_count: Option<u64> }, SendRejected { session, reason: SendRejection }, ModelMissing { session, model: String }, StoreWarning(String), SessionError { session: Option<SessionId>, message: String } }`
  - `pub enum TurnOutcome { Completed, Failed, Cancelled }`
  - `pub type EventSink = Arc<dyn Fn(ChatEvent) + Send + Sync>`
- Produces in `chat::manager`:
  - `pub struct ChatConfig { pub ollama: OllamaClient, pub flush_interval: Duration, pub coalesce: Duration }` with `Default`-style `new(ollama)` using the spec constants.
  - `pub struct ChatHandle` (cloneable) with `send(&self, ChatCommand)`; `ChatManager::spawn(rt: &tokio::runtime::Handle, writer: Writer, cfg: ChatConfig, sink: EventSink) -> ChatHandle`.

Behavior in this task (no runs yet): startup emits `Rows` from `writer.snapshot()`; `Create` makes a chat session (`Session::new_chat`, system prompt kind Chat), saved only when `Persisted`; incognito sessions live in an in-memory map and appear in `Rows` with `lifetime = Incognito` but are never given to the writer; `Open` loads (persisted via writer, then `Opened`); `Rename`/`SetPinned` go through the writer and re-emit `Rows`; rename/pin leave ordering alone; `Close` on a saved session is view-level only (no eviction while a run is active, nothing to do yet) and on incognito destroys it; `ClearIncognito` destroys all; `Delete` removes from the map first, tombstones, then deletes; `SetModel` changes the model when idle.

- [ ] **Step 1: Write failing tests** (tempdir store, collecting sink): `create_persisted_chat_appears_in_rows_and_on_disk`; `create_incognito_appears_in_rows_but_writes_nothing_to_disk` (directory listing equal before/after); `open_returns_the_transcript`; `rename_and_pin_do_not_change_updated_at_or_order`; `closing_incognito_destroys_it`; `clear_incognito_destroys_all`; `delete_removes_file_and_row`; `set_model_changes_model_when_idle`; `restart_lists_saved_sessions_again` (new manager on the same dir).
- [ ] **Step 2:** `cargo test -p roost-core chat::manager` — Expected: FAIL.
- [ ] **Step 3:** Implement the loop with `tokio::select!` over the command receiver; use a test helper `fn collect() -> (EventSink, Arc<Mutex<Vec<ChatEvent>>>)`.
- [ ] **Step 4:** Run — Expected: PASS.
- [ ] **Step 5:** Commit `feat(core): chat manager session lifecycle`.

### Task 8: Runs: send, stream, stop, retry

**Files:**
- Create: `crates/roost-core/src/chat/run.rs`
- Modify: `crates/roost-core/src/chat/manager.rs`, `crates/roost-core/src/chat/mod.rs`

**Interfaces:**
- Consumes: `OllamaClient::{show, chat_stream}`, `context::{window, build_request}`, Task 1 session helpers.
- Produces: run behavior behind `ChatCommand::{Send, Stop, Retry}` and the events in Task 7. Internal: `run::spawn_run(...)` returning a `RunHandle { stop: watch::Sender<bool>, done: JoinHandle<()> }`.

Rules: `Send` while the session has an active run → `SendRejected{Busy}` (nothing saved). Otherwise save the user message first (writer), emit `TurnStarted`, `Loading`, then request with `num_ctx = window(...)` (show cached per model name; `InstalledChanged` clears the cache; `show` failure falls back to 8,192). Too large → `SendRejected{TooLarge}`. `ContextTrimmed` emitted when `omitted > 0`. Deltas coalesce for `cfg.coalesce`, are flushed **before** the terminal event; the assistant message is appended in memory and sent to the writer every `flush_interval` and at the end. Terminal outcomes: `Done` → `Completed`; any client error, stall or early close → `Failed` (message text from the error; empty assistant message with `error` set when nothing streamed); `Stop` → `Cancelled`. `ModelNotFound` additionally emits `ModelMissing`. The manager records the first terminal for a turn; later ones are dropped. `Retry` is only valid when the last assistant message is `Failed`, `Cancelled` or `Interrupted`; it appends a new assistant message and re-sends from the last user message. `SetModel` while a run is active is rejected with a `SessionError`.

- [ ] **Step 1: Write failing tests** (TCP script for the model, wiremock for `/api/show`; shortened timeouts via config): `a_chat_streams_and_completes_and_is_saved`; `the_request_carries_num_ctx_equal_to_the_trim_window`; `a_second_send_while_running_is_rejected_busy`; `stop_cancels_and_keeps_the_partial_text_labelled_cancelled`; `stop_racing_a_held_done_line_yields_one_terminal_cancelled` (script holds the final line on a `WaitFor`; send Stop; release; assert a single `TurnEnded` and no `Delta` after it); `completion_then_stop_yields_one_terminal_completed`; `early_close_without_done_fails_and_keeps_partial_text`; `a_stall_fails_once`; `unreachable_ollama_fails_once_with_a_readable_message`; `failed_before_the_first_token_leaves_an_empty_error_message`; `a_404_emits_model_missing`; `retry_after_failure_appends_a_new_assistant_message_and_excludes_the_failed_one_from_context`; `a_crash_interrupted_last_turn_offers_retry_and_resends`; `a_new_message_after_a_failed_exchange_sends_no_consecutive_user_turns`; `long_history_is_trimmed_with_the_marker_and_the_transcript_is_untouched`; `an_oversized_message_is_rejected_too_large`; `deltas_are_coalesced_and_arrive_before_the_terminal_event`; `partial_text_is_flushed_to_disk_while_streaming` (small `flush_interval`); `two_sessions_run_concurrently`.
- [ ] **Step 2:** `cargo test -p roost-core chat::` — Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run — Expected: PASS.
- [ ] **Step 5:** Commit `feat(core): streaming chat runs with stop, retry and trimming`.

### Task 9: Delete, close and shutdown during runs; core incognito checks

**Files:**
- Modify: `crates/roost-core/src/chat/manager.rs`, `crates/roost-core/src/chat/run.rs`
- Test: `crates/roost-core/src/chat/manager.rs` (tests module) or `crates/roost-core/tests/chat_incognito.rs`

**Interfaces:**
- Produces: `ChatHandle::shutdown(&self) -> impl Future<Output = ()>` (sends `Shutdown`, completes within 3 s: cancels runs, final flush marking partial replies Cancelled, destroys incognito); delete-during-run teardown order (remove from map, tombstone, cancel, await the run task, unlink); saved-session `Close` keeps the session in the map until its run ends.

- [ ] **Step 1: Write failing tests**: `delete_during_a_run_leaves_no_file_even_with_a_late_flush`; `closing_a_running_saved_session_keeps_the_final_flush`; `closing_a_running_incognito_session_cancels_and_destroys_it`; `shutdown_flushes_a_cancelled_partial_within_the_limit`; `shutdown_destroys_incognito`; `a_full_incognito_conversation_leaves_the_data_dir_byte_identical` (listing plus contents of the whole tempdir before/after a create, send, close and clear).
- [ ] **Step 2:** Run them — Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** `cargo test -p roost-core` — Expected: PASS.
- [ ] **Step 5:** Commit `feat(core): delete, close and shutdown semantics during runs`.

### Task 10: GUI runtime wiring

**Files:**
- Modify: `crates/roost-gui/src/runtime.rs`, `crates/roost-gui/src/app.rs`, `crates/roost-gui/src/main.rs`

**Interfaces:**
- Consumes: `ChatManager::spawn`, `ChatHandle`, `Store::open`, `Writer::spawn`.
- Produces: `BackendRequest::Chat(ChatCommand)`; `UiEvent::Chat(ChatEvent)`; `UiEvent::ChatUnavailable(String)` (history locked or store unusable; Browse, Installed and Settings keep working); `ApplySettings` rebuilds the manager's Ollama client (send a `ChatCommand`-free internal update through `ChatHandle::set_ollama(OllamaClient)`); installed-list refresh sends `InstalledChanged`; on window close the app calls `ChatHandle::shutdown` with the 3-second bound before exit.

- [ ] **Step 1: Write failing tests** in `runtime.rs`: `chat_commands_round_trip_to_events` (create a chat, expect `Rows`/`Created`); `a_locked_history_reports_chat_unavailable_and_other_requests_still_work`; `chat_events_request_a_repaint`; `applying_settings_updates_the_chat_client`; `closing_the_app_flushes_a_running_chat` (extends the quit tests).
- [ ] **Step 2:** `cargo test -p roost-gui runtime quit` — Expected: FAIL.
- [ ] **Step 3:** Implement; the manager's sink forwards through the existing `Emitter`.
- [ ] **Step 4:** `cargo test --workspace` — Expected: PASS.
- [ ] **Step 5:** Commit `feat(gui): wire the chat manager into the backend`.

### Task 11: Chat view-model

**Files:**
- Create: `crates/roost-gui/src/chat/mod.rs`, `crates/roost-gui/src/chat/state.rs`
- Modify: `crates/roost-gui/src/main.rs` (`mod chat;`), `crates/roost-gui/src/app.rs` (`AppState.chat: ChatState`, fold `UiEvent::Chat`)

**Interfaces:**
- Produces:
  - `pub struct ChatState` with `fn fold(&mut self, ev: ChatEvent)`, `fn rail(&self, now_ms: u64) -> Rail`, `fn active_runs(&self) -> usize`, `fn open_session(&self) -> Option<&OpenSession>`, per-session drafts (`draft(id)`, `set_draft`), `unavailable: Option<String>`, `ollama_down: bool`, `models: Vec<String>`.
  - `pub struct Rail { pub pinned: Vec<RailRow>, pub today: Vec<RailRow>, pub yesterday: Vec<RailRow>, pub earlier: Vec<RailRow>, pub incognito: Vec<RailRow> }` (`RailRow { id, title, model, pinned, running: RunBadge, status: RowStatus }`; groups sorted by `updated_at_ms` descending; day buckets by local-day boundary passed in).
  - `OpenSession` holds the transcript plus the in-flight assistant text, folded by turn id.
  - `fn default_model(&self, installed: &[String]) -> Option<String>` (most recent session's model, else first installed alphabetically).

- [ ] **Step 1: Write failing tests**: `deltas_fold_into_the_streaming_message_by_turn`; `a_duplicate_or_late_terminal_event_is_ignored`; `terminal_labels_failed_stopped_and_interrupted_map_correctly`; `a_pinned_session_is_listed_once_under_pinned`; `rail_groups_today_yesterday_earlier_and_incognito`; `rename_and_pin_do_not_reorder_rows`; `active_run_count_covers_background_sessions`; `default_model_prefers_the_latest_session_then_alphabetical`; `enter_during_a_run_keeps_the_draft` (state-level: `try_send` returns `Rejected` and leaves the draft); `unavailable_history_is_recorded`.
- [ ] **Step 2:** `cargo test -p roost-gui chat::state` — Expected: FAIL.
- [ ] **Step 3:** Implement as plain data with no egui types.
- [ ] **Step 4:** Run — Expected: PASS.
- [ ] **Step 5:** Commit `feat(gui): chat view-model and rail grouping`.

### Task 12: Chat view, navigation and layout tests

**Files:**
- Create: `crates/roost-gui/src/chat/view.rs`, `crates/roost-gui/src/chat/layout_tests.rs`
- Modify: `crates/roost-gui/src/app.rs` (add `View::Chat` as the first tab; label `Chat`; show a small activity badge while `active_runs() > 0`), `crates/roost-gui/src/browse/mod.rs` (the `Harness` only if needed for themes)

**Interfaces:**
- Consumes: `ChatState`, `theme::tokens`, `confirm.rs`, `widgets.rs`.
- Produces: `pub fn show(ui: &mut egui::Ui, st: &mut AppState, tx: &Sender<BackendRequest>, now: Instant)`.

Content per the spec's GUI section: rail with New chat and New incognito, Pinned/Today/Yesterday/Earlier groups, a live Incognito group with a memory-only note and Clear, row actions (open, rename, pin, delete with `confirm.rs`), errored and newer-schema rows delete-only; conversation pane with message cards, collapsed thinking, terminal labels, Retry on the last failed/stopped/interrupted turn, the `N earlier messages omitted` note, model picker, composer (Enter sends, Shift+Enter newline; during a run Send becomes Stop, Enter does nothing, draft kept), incognito banner, banner with Retry when Ollama is down, and a pointer to Browse when no models are installed; "Another Roost is using this history" notice when unavailable.

**Also in this task (user request):** the top bar's logo, tabs, search input and Ollama status tag are not vertically aligned. Fix them to share one vertically centered row, with a layout test `top_bar_items_share_one_vertical_center` (every theme, 900x640) asserting their rect centers agree within 1 px.

- [ ] **Step 1: Write failing tests**: `every_theme_family_and_mode_keeps_chat_usable_at_minimum_size` (long titles, model names, messages, many rows); `chat_is_the_first_tab_and_default_view_when_history_is_available`; `the_nav_shows_activity_while_a_background_run_is_active`; `send_button_becomes_stop_during_a_run`; `enter_does_not_send_during_a_run`; `delete_needs_the_confirm_pause`; `incognito_banner_and_clear_are_shown`; `errored_rows_offer_delete_only`.
- [ ] **Step 2:** `cargo test -p roost-gui chat` — Expected: FAIL.
- [ ] **Step 3:** Implement following the existing views' structure (`installed/mod.rs`).
- [ ] **Step 4:** `cargo test --workspace` — Expected: PASS, existing view tests green.
- [ ] **Step 5:** Commit `feat(gui): chat view with history rail`.

### Task 13: GUI incognito leak checks

**Files:**
- Test: `crates/roost-gui/src/chat/leak_tests.rs`
- Modify: `crates/roost-gui/src/main.rs` / `crates/roost-gui/src/app.rs` only if a leak is found

**Interfaces:**
- Consumes: Task 11 and Task 12.

- [ ] **Step 1: Write failing tests**: `incognito_text_never_appears_in_saved_egui_memory` (serialize `ctx.memory()` / eframe's persisted state after typing a draft and rendering a conversation; the secret string must be absent); `closing_incognito_drops_its_draft_and_transcript_state`; `ui_events_for_incognito_are_not_logged` (capture the app's log output while folding a full incognito conversation).
- [ ] **Step 2:** Run — Expected: FAIL if a leak exists, otherwise PASS immediately; in that case also add a mutation check (temporarily write the draft into persisted state and see the test fail) and record it in the ledger.
- [ ] **Step 3:** Fix any leak found (clear the draft and open-session state on close; keep incognito text out of any `ctx.data` that eframe persists).
- [ ] **Step 4:** `cargo test --workspace` — Expected: PASS.
- [ ] **Step 5:** Commit `test(gui): incognito text never reaches persisted or logged state`.

### Task 14: Docs and final gates

**Files:**
- Modify: `README.md`, `CHANGELOG.md`, `docs/ARCHITECTURE.md`, `docs/ROADMAP.md`, `docs/DEVELOPMENT.md`, `docs/superpowers/notes/handoff.md`, `scripts/check_roost_identity.py` only if the scan needs a narrow allowance

- [ ] **Step 1:** Update the docs: chat and history are implemented (stage 2), storage layout and single-instance lock, `num_ctx` behavior and the 16,384 cap, deferred items (markdown, copy, regenerate, search, Manage, agents), handoff update.
- [ ] **Step 2:** Run `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, the same with `--features dev-hooks` as CI does, `cargo test --workspace`, `python3 scripts/check_roost_identity.py`, `python3 -B scripts/test_check_roost_identity.py`, `cargo deny check`, `git diff --check` — Expected: all clean.
- [ ] **Step 3:** Commit `docs: roost stage 2 chat and history`.
