# Roost Stage 2: Streaming Chat, Persistence and History Rail

**Status:** Draft for review

**Date:** 2026-10-27

**Parent spec:** `2026-10-27-roost-workbench-design.md` (stage 2 of its delivery sequence). That spec stays binding wherever this one is silent.

## Summary

Stage 2 makes Roost chat. A user can create a saved chat, stream a reply from any installed Ollama model, close Roost, reopen it and continue. An incognito chat exists only in memory. A history rail lists sessions, with pinning. Runs in different sessions continue in the background.

Stage 2 adds no agents, tools, steering, `/btw`, branches, auto-title, history search or filters, history manager, attachments or summarization. Stages 3 and 4 add to the interfaces defined here; they do not rewrite them.

Acceptance criteria from the parent spec this stage must satisfy: 2, 3, 11, 12, 13, and the stage-2 parts of 1 and 17.

## Decisions taken

- The chat runtime lives in `roost-core` as a single **session manager** task. `roost-gui` sends intent commands and folds ordered events. The GUI does no HTTP or file work. Existing `runtime.rs` is only wired to forward to and from the new channels.
- The rail includes pinning (the `Session::pinned` field already exists). It excludes search, filters and Manage.
- Chat works with any installed model. The `tools` capability is read and cached now but only gates features in stage 3.

## Storage (`roost-core::store`)

Location: `Paths::sessions_dir`. In portable mode that is `sessions/` beside the executable.

- `index-v1.json`: metadata only. One row per persisted session: id, title, mode, lifetime, pinned, model, parent link, created and updated time, a run-state summary, and a `load_error` flag.
- `<session-id>.json`: one `SessionDocument` (schema version 1, already defined in `session.rs`).
- Every write uses `persistence::atomic_write`. Saving a message rewrites that session's file and then the index. It never touches other session files.
- The index is derived data. If it is missing, unreadable or has an unsupported version, the store rebuilds it by scanning the session files and reports a recoverable warning.
- A session file that fails to parse, validate (`into_current`) or has an unsupported schema is renamed to `<session-id>.json.corrupt-<unix-ms>` and appears in the rail as an errored row that offers Delete. It never panics and never blocks other sessions.
- On load, `Session::normalize_after_crash` settles any session left in a running state. A partial assistant message is kept and labelled.
- Incognito sessions are held only in the manager's memory. The store API cannot receive them. Nothing about them reaches the index, recovery files, recent-session metadata or logs. Closing one, clearing incognito or exiting destroys it without a prompt.
- The mutation API is narrow: `create`, `save_session`, `rename`, `set_pinned`, `delete`, `load`, `list`.

## Streaming (`OllamaClient::chat_stream`)

- Streams `POST /api/chat` as newline-delimited JSON. Chunks may split lines arbitrarily; the decoder buffers until a full line. A malformed line ends the turn as Failed with a readable message.
- Yields content deltas, thinking deltas (kept separate from content), and a final event carrying token counts and the done reason.
- Reuses the client's existing stall timeout. Adds a first-event timeout of 5 minutes, shown to the user as `Loading model…` until the first event.
- Requires enabling reqwest's streaming support. No OpenSSL; rustls only, as today.
- `show` results are cached by model digest, with context length and capabilities. The cache is refreshed when the installed models list changes.

## Session manager (`roost-core::chat`)

One manager task owns: the open and persisted sessions, the incognito map, the store, the index, and the rule that **at most one run is active per session**. Runs in different sessions may overlap; Ollama may queue them, and the UI says `Waiting for model` rather than implying simultaneity.

Commands (intents):

- `Create { kind: Chat | Incognito, model }`
- `Open(SessionId)`, `Close(SessionId)` (closing incognito destroys it)
- `Rename`, `SetPinned`, `SetModel`, `Delete` (stops any active run first)
- `Send { session, text }`, `Stop(session)`, `Retry(session)`
- `ListSessions`, `ClearIncognito`

Events are ordered per session and carry `SessionId`, `RunId` and `TurnId`:

- session list and row changes, and `Opened` with the transcript
- `TurnStarted`, `Loading`, `WaitingForModel`, `Delta { content | thinking }`, and exactly one terminal event per turn: `Completed`, `Failed { message }`, `Cancelled` or `Interrupted`
- `ContextTrimmed { omitted }`, `StoreWarning`, `SessionError`

Rules:

- A user message is saved before the request is sent.
- The assistant message grows in memory and is flushed to disk about every 2 seconds and at the terminal event. A crash therefore loses at most about 2 seconds of text.
- Each `Send` spawns one run task. `Stop` is explicit cancellation; terminal events are idempotent, so a late event after the terminal one is dropped.
- A failed or cancelled turn keeps its partial output, labelled with its terminal state. `Retry` re-sends from the last user message without rewriting earlier messages.
- Unreachable Ollama, a missing model, a stall and a load timeout each end the turn once with a readable message. The transcript and composer stay usable. A missing model prompts selection of another installed model.
- Titles stay `Untitled` until the user renames. Auto-title is stage 4. The `title_metadata` fields stay in the format so stage 4 needs no migration.

## Context management (pure functions)

Applied before each request, exactly as the parent spec states:

- Window: the model's context length from `show`, else 8,192 tokens.
- Estimate: `ceil(UTF-8 bytes / 3) + 8` per message.
- Reserve 30% of the window for output. Send the newest complete message groups that fit.
- Always keep the system prompt and the newest user message.
- When older messages are dropped, prepend the synthetic `Earlier messages omitted for context` marker and report the omitted count. The local transcript is never trimmed.
- If the system prompt plus the newest user message do not fit, reject the send with a readable size error.

Message groups are the unit of trimming so stage 3 can keep tool call and result pairs indivisible without changing this function's signature.

## GUI (`roost-gui`)

- **Chat view:** a left rail and a conversation pane. No session tab strip.
- **Rail:**
  - New chat and New incognito actions. New agent and Manage are absent in this stage.
  - Pinned group first, then Today, Yesterday and Earlier. A pinned session appears only under Pinned.
  - A separate, live Incognito group, with a memory-only note and Clear.
  - Rows show title (truncated), model, and a background-run indicator (spinner or `Waiting for model`). Row actions: open, rename, pin or unpin, delete. Delete uses the existing confirmation pattern in `confirm.rs`. Errored rows offer Delete only.
- **Conversation pane:**
  - Message cards. Streaming text updates live. Partial replies carry a terminal label (Interrupted, Failed, Stopped). Thinking output is shown separately and collapsed.
  - A Retry action on a failed turn, and an `N earlier messages omitted` note.
  - A model picker fed by the installed models.
  - A composer: Enter sends, Shift+Enter inserts a newline. While a run is active, Send becomes Stop. Typing is still allowed; steering is stage 3.
  - A visible incognito banner.
- **State:** a view-model folds manager events by ID and is kept apart from rendering, like the existing views. The top bar's activity area and the Chat nav item show background runs while the user is in Browse, Installed or Settings. Switching views or sessions never stops a run.
- **Visuals:** colors come only from theme tokens. Layout tests render the Chat view in all four families, light and dark, at the minimum window size 900x640, with long titles, model names and messages.

## Testing

All tests are test-first and use wiremock and tempdirs. Tests never touch port 11434, the real keychain or the real config.

Core:

- Streaming decode: split chunks, multiple lines per chunk, thinking and content interleaved, malformed line, early close.
- Exactly one terminal event per turn, including the cancel-versus-complete race.
- Cancel while streaming. Stall timeout. Load timeout.
- Partial flush after a simulated crash, and recovery with `normalize_after_crash`.
- Store: per-session rewrite does not touch other files; corrupt session file is quarantined; corrupt, missing or unsupported-version index is rebuilt; pin persists across an index rebuild.
- Incognito writes nothing: a recursive listing and content check of the temp directory before and after a full incognito conversation, a close, and a clear.
- Context trimming boundaries: exact fit, one byte over, only-system-plus-newest, the fallback window, the omitted marker and count.
- One active run per session; two sessions run concurrently.

GUI:

- Event folding, including out-of-order and duplicate terminal events.
- Rail grouping and pinning logic (a pinned session is listed once).
- Headless layout in every theme, light and dark, at minimum size.
- Existing Browse, import, Installed, Settings and job-dock tests stay green.

Docs: README, CHANGELOG, ARCHITECTURE and ROADMAP are updated in the same stage, as stage 1 did. The fmt, clippy `-D warnings`, identity scan and `cargo deny` gates stay green.

## Out of scope

Agents, tools, steering, `/btw`, branches, auto-title, rail search and filters, the history manager, attachments, summarization, a second content index, and any persistence of incognito data.

## Risks

- **Streaming plus cancellation** is the hardest part. The one-terminal-event rule and the race tests are there to pin it down.
- **Disk format.** Schema v1 is now what users' history is written in. Any field added later must be additive with a serde default, and the migration tests must cover older files.
- **GUI repaint cost** from token-rate events. The view-model coalesces deltas per frame.
