# Roost Stage 2: Streaming Chat, Persistence and History Rail

**Status:** Revised after review, awaiting approval

**Date:** 2026-10-27

**Parent spec:** `2026-10-27-roost-workbench-design.md` (stage 2 of its delivery sequence). That spec stays binding wherever this one is silent.

## Summary

Stage 2 makes Roost chat. A user can create a saved chat, stream a reply from any installed Ollama model, close Roost, reopen it and continue. An incognito chat exists only in memory. A history rail lists sessions, with pinning. Runs in different sessions continue in the background.

Stage 2 adds no agents, tools, steering, `/btw`, branches, auto-title, history search or filters, history manager, attachments or summarization. Stages 3 and 4 add to the interfaces defined here; they do not rewrite them.

Parent acceptance criteria this stage must satisfy: 2, 3, 11, 12, 13, and the stage-2 parts of 1 and 17.

## Decisions taken

- The chat runtime lives in `roost-core` as a single **session manager** task. `roost-gui` sends intent commands and folds ordered events. The GUI does no HTTP or file work. `runtime.rs` is only wired to forward to and from the new channels.
- The rail includes pinning (`Session::pinned` already exists). It excludes search, filters and Manage.
- Chat works with any installed model. The `tools` capability is not read in this stage; stage 3 adds it.

## Context window (`num_ctx`)

`/api/show` reports the model's **maximum** context, not what Ollama loads by default. Trimming against the maximum would let Ollama truncate the prompt silently on the server, so the trimming would protect nothing. Therefore:

- Every chat request sends an explicit `options.num_ctx`.
- Ollama's own default window is smaller than the model maximum and varies by version and available memory. That was not verified against a live server here (tests never use the real Ollama), but sending an explicit value is correct either way.
- The window is `session.model_options.num_ctx` if set, else `min(show context, 16,384)`, else 8,192 when `show` gives no context. The 16,384 cap keeps KV-cache memory reasonable for models that advertise 128k or more. The cap is a named constant.
- Trimming budgets against that same number, so what Roost trims to is what Ollama loads.
- The final stream event's `prompt_eval_count` is surfaced in the event (for display and debugging). Using it to recalibrate the estimate is deferred.

## Storage (`roost-core::store`)

Location: `Paths::sessions_dir`. In portable mode that is `sessions/` beside the executable.

- `index-v1.json`: metadata only. One row per session: id, title, mode, lifetime, pinned, model, parent link, created and last-activity time, a run-state summary, and a load status (ok, quarantined, newer-schema, unreadable).
- `<session-id>.json`: one `SessionDocument` (schema version 1, already defined in `session.rs`).
- Every write uses `persistence::atomic_write`. Saving a message rewrites that session's file, then the index, and only when the row's metadata actually changed. It never touches other session files.
- **The index is derived data.** On startup the store reconciles it with the directory: a session file missing from the index is added; a row with no file is dropped; a row whose recorded file mtime and size differ from the file is refreshed. **File mtime and size are the staleness signal**; `updated_at` is only for rail ordering, because rename and pin deliberately leave it alone. A missing, unreadable or unsupported-version index is rebuilt by the same scan. Rebuilds report a recoverable warning. This also covers a crash between the session write and the index write.
- **Load failures are classified, not all quarantined:**
  - Parse or validation failure (`SessionDocument` / `into_current`): rename to `<session-id>.json.corrupt-<unix-ms>` and list as an errored row offering Delete. The scan also lists existing `*.corrupt-*` files, so errored rows survive an index rebuild.
  - A file written by a **newer** schema is left untouched and shown read-only as "Made by a newer Roost", so a downgrade never damages it.
  - A transient I/O error (permissions, a locked file) is not renamed. The row shows as unreadable and is retried on the next list.
  - (`check_workspace` is syntactic only. A missing workspace folder does not fail `into_current`, and stage 2 has no workspaces anyway.)
- `normalize_after_crash` runs **at startup load only**, never on `Open` of a session the manager is currently running.
- **One instance per history.** The store takes an exclusive advisory lock on `sessions/.lock` (`File::try_lock`). A second Roost on the same directory starts with chat disabled and a plain-language notice ("Another Roost is using this history"); Browse, Installed and Settings still work.
- Incognito sessions live only in the manager's memory. The store API cannot receive them. Nothing about them reaches the index, recovery files, recent-session metadata or logs.
- Rename and pin do **not** change `updated_at`. Only message activity does, so the rail does not reshuffle when the user tidies up. `Session::rename` currently sets `updated_at_ms` (and a test asserts it); that method changes so it no longer does, and its `now_ms` parameter goes away. The index row records the file's mtime and size at each write.
- Disk work never runs on the manager task. Writes go to a small writer task using `spawn_blocking`, so a slow disk cannot stall deltas. Writes for a session id no longer in the manager's map are dropped.
- Additive schema change: the assistant message gains an optional `thinking` text (serde default, schema stays v1) so reasoning deltas survive restart. Thinking is shown collapsed and is **never** sent back in a request.
- The store's mutation API is narrow: `create`, `save_session`, `rename`, `set_pinned`, `delete`, `load`, `list`.

## Streaming (`OllamaClient::chat_stream`)

- Streams `POST /api/chat` as newline-delimited JSON. The decoder buffers **bytes**, not strings, and splits on newlines, so a multi-byte UTF-8 character split across chunks decodes correctly.
- Yields content deltas, thinking deltas (kept separate) and a final event with token counts and done reason.
- Timeouts: a first-event timeout of 5 minutes (the UI shows `Loading model…` until then), and a new `chat_stall` of 120 seconds between events once streaming has started. `with_stall_timeout` overrides `chat_stall` too, and a separate `with_first_event_timeout` overrides the 5-minute limit, so tests of both stay fast. (The client today has only upload and create stalls; reqwest's `stream` feature is already enabled.)
- `chat_stream` reuses the client's `check` and `error_message` for HTTP errors and in-stream `{"error": ...}` lines, and parses its own line type (the existing `handle_line` is specific to `create`). `check` currently drops the status code when the body is a JSON error, so it gains a variant that keeps the status; that is how a 404 becomes "model not found" instead of a generic error.
- **Ollama's error shapes**, all of which end the turn as Failed with a readable message:
  - a non-200 response with a JSON `{"error": "..."}` body, where 404 maps to a distinct "model not found" outcome that drives the reselect prompt;
  - a 200 stream that carries a valid `{"error": "..."}` line mid-stream;
  - a malformed line;
  - a stream that closes without `done: true` ("connection ended early").
- `show` results are cached by model digest (context length) and refreshed when the installed list changes.

## Session manager (`roost-core::chat`)

One manager task owns the open and persisted sessions, the incognito map, the index and the rule that **at most one run is active per session**. Runs in different sessions may overlap. Ollama may queue them, and the UI says `Waiting for model` rather than implying simultaneity.

Commands (intents):

- `Create { kind: Chat | Incognito, model }`, `Open`, `Close`, `ClearIncognito`. For a saved session, `Close` is view-level only: it never evicts the session or stops a run, and a session with an active run stays in the manager's map until the run ends and its final flush is written. Closing an incognito session cancels any run and destroys it.
- `Rename`, `SetPinned`, `Delete`
- `SetModel` (rejected while a run is active; allowed when idle even if the old model was removed)
- `Send { session, text }`, `Stop(session)`, `Retry(session)`
- `ListSessions`

Events are ordered per session and carry `SessionId`, `RunId` and `TurnId`: row changes, `Opened` with the transcript, `TurnStarted`, `Loading`, `WaitingForModel`, `Delta { content | thinking }`, `ContextTrimmed { omitted }`, `StoreWarning`, `SessionError`, `SendRejected { reason }`, and exactly one terminal event per turn.

**Terminal outcomes in stage 2:**

- `Completed`: `done: true` received.
- `Failed { message }`: any error shape above, including a stream that ended early or stalled.
- `Cancelled`: the user pressed Stop, or Roost is quitting.
- `Interrupted` is not emitted live in stage 2. It exists in the format for crash recovery (`normalize_after_crash`) and is reserved for stage 3 steering.

**Rules:**

- A user message is saved before the request is sent.
- The assistant message grows in memory and is flushed through the writer about every `FLUSH_INTERVAL` (default 2 seconds, a tunable constant, not a requirement) and at the terminal event.
- Each `Send` spawns one run task. Terminal events are idempotent: a late event after the terminal one is dropped. The cancel-versus-complete race resolves to whichever the manager records first, and only that one is emitted.
- **Send while a run is active** is rejected with `SendRejected { Busy }`. In the GUI, Enter does nothing during a run. The draft is kept for sending afterwards. Typing stays enabled so the user can prepare the next message.
- **Retry** is offered on the last assistant message when it is Failed, Cancelled or **Interrupted** (a partial reply left by a crash and marked at the next startup, so a restart after a crash always has a way forward). The failed message stays in the transcript. Retry re-sends from the last user message and appends a **new** assistant message. A failed, cancelled or interrupted assistant message is excluded from the next request's context. A turn that fails before the first token still creates an empty assistant card carrying the error, so Retry always has somewhere to attach.
- **Delete during a run:** the manager removes the session from its map first (so no later write can recreate the file), cancels the run, awaits the run task's teardown, then unlinks the file and updates the index.
- **Quit with active runs:** the app cancels all runs without a prompt, the manager does a final flush marking partial replies Cancelled, and shutdown waits for it for at most 3 seconds. The 2-second window is closed on exit. Incognito sessions are destroyed.
- `SetModel` to a removed model shows the picker's missing-model state. It never rewrites earlier messages.
- Unreachable Ollama, a missing model, a stall and a load timeout each end the turn once with a readable message. The transcript and composer stay usable.
- Titles stay `Untitled` until the user renames. Auto-title is stage 4; `title_metadata` stays in the format so stage 4 needs no migration.
- **Channels:** the manager coalesces content deltas in a short window (about 30 ms) before sending, so the queue is bounded by time rather than token rate even when the window is not repainting. Any pending coalesced delta is flushed **before** the terminal event is sent, so no text arrives after its turn has ended. Events use an unbounded channel; the runtime calls `request_repaint` after forwarding.

## Context management (pure functions)

Applied before each request, as the parent spec states, against the window defined above:

- Estimate: `ceil(UTF-8 bytes / 3) + 8` per message. Reserve 30% of the window for output. Send the newest complete groups that fit.
- A **group** in stage 2 is a user message plus the assistant reply that followed it. A request therefore never begins with an orphaned assistant message. Stage 3 extends a group to include tool call and result pairs without changing the function's signature.
- Thinking text is never sent. An exchange whose reply did not complete (Failed, Cancelled or Interrupted) is dropped **whole**, user message included, whenever a later user message exists. If the user types a new message instead of pressing Retry, the request therefore never contains two user turns in a row. The transcript keeps the dropped exchange. (When the failed exchange is the latest one, it is the newest user message and is kept, which is what Retry sends.)
- Always keep the system prompt and the newest user message.
- Dropped older groups are replaced by the synthetic `Earlier messages omitted for context` marker, and the omitted count is reported. The local transcript is never trimmed.
- If the system prompt plus the newest user message do not fit, the send is rejected with a readable size error.

## GUI (`roost-gui`)

- **Chat view:** a left rail and a conversation pane. No session tab strip.
- **Rail:**
  - New chat and New incognito. New agent and Manage are absent in this stage.
  - A Pinned group first, then Today, Yesterday and Earlier, by last activity. A pinned session appears only under Pinned. Earlier is unbounded; pagination is deferred.
  - A separate live Incognito group with a memory-only note and Clear.
  - Rows show a truncated title, model, and a background-run indicator (spinner or `Waiting for model`). Row actions: open, rename, pin or unpin, delete (confirmed via the `confirm.rs` pattern). Errored and newer-schema rows offer Delete and, for newer-schema, no editing.
- **Conversation pane:**
  - Message cards with plain selectable text. Streaming updates live. Partial replies carry their terminal label (Failed, Stopped, Interrupted). Thinking text is shown collapsed.
  - Retry on a failed, stopped or interrupted last turn, and an `N earlier messages omitted` note.
  - A model picker fed by the installed models.
  - A composer: Enter sends, Shift+Enter inserts a newline. While a run is active, Send becomes Stop, Enter does nothing, and typing is still allowed (draft kept).
  - A visible incognito banner.
- **Defaults and empty states:** New chat uses the model of the most recent session, else the first installed model alphabetically. If Ollama is unreachable, the view shows a banner with Retry. If no models are installed, it points to Browse. Both leave existing transcripts readable.
- **State:** a view-model folds manager events by ID, apart from rendering, like the existing views. The top bar's activity area and the Chat nav item show background runs while the user is in Browse, Installed or Settings. Switching views or sessions never stops a run.
- **Visuals:** colors come only from theme tokens. Layout tests render the Chat view in all four families, light and dark, at 900x640, with long titles, model names and messages.
- **Deferred, not forgotten:** markdown and code-block rendering, a copy button, and a regenerate action on completed replies.

## Testing

All tests are test-first and use wiremock, a raw TCP test server and tempdirs. Tests never touch port 11434, the real keychain or the real config.

Core:

- Streaming decode against a **raw TCP server** (wiremock cannot split a body at arbitrary bytes or stall mid-stream): split chunks, several lines per chunk, a multi-byte UTF-8 character split across chunks, interleaved thinking and content, stall mid-stream (ends Failed), early close without `done` (ends Failed, not Interrupted).
- Error shapes: non-200 JSON error, 404 model-missing, mid-stream `{"error"}` line, malformed line.
- Exactly one terminal event per turn. The cancel-versus-complete race uses a deterministic injection seam (a gate on the stream source), not timing.
- First-event timeout and `chat_stall`, using their shortened overrides (`with_first_event_timeout`, `with_stall_timeout`).
- A crash-recovered Interrupted last turn offers Retry and sends the right request; a new message after a failed exchange yields a request with no consecutive user turns; the coalesced delta arrives before the terminal event; rename and pin leave `updated_at` and ordering unchanged; Close on a running saved session keeps the final flush.
- Partial flush after a simulated crash and recovery with `normalize_after_crash`.
- Store: a per-session rewrite does not touch other files; corrupt files are quarantined; a newer-schema file is untouched; a transient I/O error does not rename; corrupt, missing, stale or unsupported-version index is reconciled; quarantined rows survive a rebuild; pins survive a rebuild; a second instance is refused by the lock.
- Delete during an active run leaves no file behind, even with a late flush.
- Send during a run is rejected. Two sessions run concurrently. Retry excludes the failed message from context and appends a new one.
- Quit with an active run flushes a Cancelled partial within the time limit.
- `num_ctx` is present in every request body and equals the window used for trimming.
- Trimming boundaries: exact fit, one byte over, only system plus newest, the fallback window, the 16,384 cap, the marker and count, no orphaned assistant message.
- **Incognito leaks:** a recursive listing and content diff of the data directories before and after a full incognito conversation, a close and a clear; captured log output; GUI composer and draft state; and eframe persistence (the test fails if incognito text appears in any saved egui memory).

GUI:

- Event folding, including duplicate and late terminal events.
- Rail grouping and pinning (a pinned session is listed once), and no reshuffle on rename or pin.
- Headless layout in every theme, light and dark, at minimum size.
- Existing Browse, import, Installed, Settings and job-dock tests stay green.

Docs: README, CHANGELOG, ARCHITECTURE and ROADMAP are updated in the same stage. The fmt, clippy `-D warnings`, identity scan and `cargo deny` gates stay green.

## Out of scope

Agents, tools, steering, `/btw`, branches, auto-title, rail search and filters, the history manager, attachments, summarization, a second content index, markdown rendering, regenerate, and any persistence of incognito data.

## Risks

- **Streaming plus cancellation** is the hardest part. The one-terminal-event rule, the injection seam and the race tests pin it down.
- **Disk format.** Schema v1 is what users' history is written in. Any later field must be additive with a serde default, and tests must load older files.
- **Window size.** An explicit `num_ctx` raises memory use on large models. The 16,384 cap and the per-session override bound it. If users hit limits, the cap becomes a setting.
