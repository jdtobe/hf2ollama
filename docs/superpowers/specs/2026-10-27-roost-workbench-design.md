# Roost Local Model Workbench Design

**Status:** Approved product design

**Date:** 2026-10-27

## Summary

Roost evolves hf2ollama from an importer into a local-model workbench. It keeps the existing Hugging Face browsing, import, and installed-model workflows, and adds persistent chat, read-only agents, incognito sessions, side conversations, branches, automatic session titles, and history management.

Roost remains a native Rust application built with egui/eframe. The core crate owns HTTP, persistence, session behavior, agent execution, tools, and migration. The GUI renders state and sends commands; it does not perform HTTP or filesystem tool work directly.

## Product identity

- The product name is **Roost**.
- The primary mark is **The Tail**, a geometric rooster-tail mark rather than a cartoon mascot.
- The main navigation is **Chat › Browse › Installed (#) › gear icon**.
- The product ships four theme families, each with light and dark variants: **Meadow**, **Ember**, **Plum**, and **Tide**.
- The app retains the density, typography, cards, rails, status treatments, and interaction patterns of the existing hf2ollama UI. This is an evolution, not a wholesale redesign.

## Goals

1. Chat with any locally installed Ollama model using a streaming interface.
2. Turn a chat into a read-only agent by attaching a workspace folder and tools.
3. Keep agent execution understandable through visible model, tool, and status steps.
4. Let a user steer a running agent without starting a second loop.
5. Persist normal sessions locally and provide genuinely ephemeral incognito sessions.
6. Support quick `/btw` side conversations and durable branches from any completed point.
7. Make the tool registry extensible by resource family, capability, and permission.
8. Preserve existing import behavior and migrate existing hf2ollama settings and credentials safely.

## Non-goals for the first Roost release

- A plugin marketplace or downloadable tool ecosystem.
- Write, shell, network, Git, MCP, clipboard, app-control, or model-management tools.
- Automatic permission for any future write, execute, or network capability.
- Sharing, cloud sync, accounts, remote history, or multi-user collaboration.
- File, image, or document attachments in chat; the agent workspace folder is the only first-release attachment.
- Replaying tool calls when a conversation is branched.
- A live fork whose parent and child continue sharing state.
- Model-authored conversation summarization or compaction. The first release applies deterministic context truncation while preserving the full local transcript.
- Reading sensitive credential files through tools. A later approval design may make this configurable.

## Application shell

### Navigation

The top bar contains `Chat`, `Browse`, `Installed (#)`, and a gear icon for Settings. Settings is never a text tab. Chat uses a single left history rail. There is no top session-tab strip.

Switching views or sessions does not stop a running agent. The history rail and top status area indicate background activity.

### Chat rail

The Chat rail contains:

- New chat, new agent, and new incognito actions.
- Search and filter controls.
- Pinned sessions, then chronological groups.
- A separate live Incognito group.
- A `Manage` action that opens the dedicated history manager.

A pinned session appears only in the Pinned group, not again in its chronological group.

## Session model

### Kinds and lifetime

Roost has four user-visible session forms:

- **Chat:** persisted local conversation without a workspace requirement.
- **Agent:** persisted conversation with a workspace, tool set, run state, and step budget.
- **Branch:** persisted independent session created from a completed point in another persisted session.
- **Incognito:** memory-only chat or agent that is destroyed when closed or when Roost exits.

`/btw` is an ephemeral side thread associated with a session, not a fifth persisted kind.

Every session owns its transcript, model selection, model options, system-prompt kind, title metadata (including last title time and undo value), timestamps, optional workspace, effective tool configuration, step budget, run state, and optional parent link. Exactly one agent run may be active per session. Runs in different sessions may coexist and continue in the background.

The default agent step budget is **20**. Chat's system prompt is: `You are Roost, a helpful assistant running locally. Be concise, be honest about uncertainty, and do not claim to have used tools unless a tool result appears in this conversation.` Agent mode appends: `You are operating as a read-only agent inside the attached workspace. Use only the provided tools, stay within the workspace, never invent tool results, honor steering messages, and stop when the task is complete or the step budget is exhausted.` The runtime appends the concrete workspace, enabled tools, and remaining step budget as structured context. Model `think` behavior is left unset so Ollama/model defaults apply; Roost preserves reasoning deltas separately when a model returns them but does not expose a first-release thinking-budget control.

### Persistence

Normal chats, agents, and branches use a per-session store under `sessions/`: `index-v1.json` contains metadata only, and `<session-id>.json` contains one transcript. A message update atomically rewrites only that session file, then updates the small metadata index. Corrupt files are preserved for diagnosis and produce recoverable per-session UI errors rather than an app panic.

History search reads candidate persisted session files on a backend worker and streams matching session IDs to the GUI; it does not maintain a second content index in the first release. Search is cancellable, bounded, and never reads incognito state. This avoids rewriting one monolithic session database on every token or message.

Incognito data never enters the session store, search index, recovery files, recent-session metadata, or content-bearing logs. Incognito titles, transcripts, side threads, branches, and tool results remain in memory only. Closing an incognito session, clearing incognito, or exiting Roost destroys them without a save prompt, trash, or recovery path.

An incognito branch inherits incognito lifetime. There is no `Save to history` action.

### Titles

New sessions begin as `Untitled`. Automatic title generation is controlled by an **Auto-title** checkbox per session.

When enabled, Roost may request a title:

- after the first completed exchange;
- when an agent run completes; or
- after at least four additional user turns.

The initial title may run as soon as the first exchange completes. Later retitles have a ten-minute minimum interval and run only when the session is idle. The request receives the current title and the most recent six to eight text messages, excludes tool output, and exposes only the `session.set_title(title)` host action. The model keeps the existing title by completing without calling the action. It has a small output budget.

Host actions are internal application actions, not workspace tools and not members of Files, Shell, or Network families. `session.set_title` auto-runs silently and exposes a small Undo affordance. Auto-title is unavailable for a model that does not advertise tool support.

Manual rename turns Auto-title off. Re-enabling the checkbox permits future title updates. Branches have their own Auto-title state and title. `/btw` has no title until promoted to a branch. Incognito title generation remains memory-only.

Title work never pauses, interrupts, or delays user work. Title requests enter a low-priority queue only when no user or agent model request is active or queued. New user work cancels a pending title request before dispatch. If Ollama is busy for another reason, the title remains pending until the scheduler is idle.

## Chat behavior

### Streaming

The core Ollama client supports streaming `/api/chat`. The runtime forwards ordered deltas and terminal events to the GUI. A turn has exactly one terminal outcome: completed, failed, cancelled, or interrupted.

Partial assistant output remains visible after interruption or failure and is labeled with its terminal state.

### Model capability and failure handling

Roost reads `/api/show` before enabling agent mode. The selected model must advertise the `tools` capability. A model without it remains available for chat, but Agent, `/btw` tools, and Auto-title host actions are disabled with a plain-language explanation. Capability status is cached by model digest and refreshed when Installed models change.

An unloaded model is normal: the session shows `Loading model…` until the first response event, with a five-minute first-event timeout. If Ollama is unreachable, the model is removed, loading fails, or generation stalls, the transcript and composer remain available; the turn ends once with a readable failure and Retry action. A missing model prompts selection of another installed model without rewriting prior messages.

### Context management

Roost uses the selected model's context length from `/api/show`, falling back conservatively to **8,192 tokens** when absent. Before each request it estimates prompt usage as `ceil(UTF-8 bytes / 3) + 8 tokens per message`, reserves 30% of the window for model output and tool-loop growth, and sends the newest complete message groups that fit. The system prompt and newest user message are always retained. Tool call/result pairs are indivisible.

The full local transcript is never deleted by context trimming. When older messages are omitted from a request, the model receives a synthetic `Earlier messages omitted for context` marker and the UI shows how many messages were omitted. If the system prompt plus newest user message cannot fit, Roost rejects the send with a readable size error. There is no automatic summarization in the first release.

### Composer and steering

The composer remains enabled while an agent runs. Sending during a run steers that same run:

1. The message appears immediately and is marked as steering input.
2. The active model generation is cancelled.
3. Its partial assistant response remains in the transcript marked `Interrupted by you`.
4. The steering message enters the run inbox.
5. The agent applies it at the next safe boundary and continues the same run.

There is no second loop. If a read-only tool call is already executing, it may finish before the steering input is applied. `Stop` is the hard cancellation action.

### Background execution

Switching sessions, opening Settings, Browse, Installed, or History does not stop a run. The active-session row and global status surface show background work. Roost does not pause the main run for `/btw`, branches, or automatic titles. Concurrent local-model requests may be queued by Ollama; Roost displays `Waiting for model` rather than pretending they are simultaneous.

## Agent loop

An agent is a chat session plus:

- one attached workspace folder;
- an effective tool set;
- a visible model/tool step loop;
- a configurable step budget; and
- one active run at a time.

The loop alternates model turns and tool results until completion, stop, failure, or the step budget. It checks for steering input before each model request and after each tool result. Tool calls and results are visible and expandable.

Agent creation and Start remain disabled until `/api/show` confirms the model advertises `tools`. Losing tool capability or the model during a saved session does not delete the session; it returns to an idle, blocked state until a capable model is selected.

The workspace can be changed while idle. An active run must be stopped before changing its workspace.

## Tool registry

### Model

The registry has four layers:

1. **Family:** resource domain, such as Files, Shell, Network, Git, Models, or MCP.
2. **Capability:** operation class within a family, such as Read, Write, Execute, or Fetch.
3. **Permission:** `Auto`, `Ask`, or `Off`.
4. **Tool:** a concrete callable action that belongs to exactly one family and capability.

Global defaults produce an effective per-session configuration. A session may override a whole capability or a single tool without deleting it from the registry. A running agent sees changes at the next safe step; an in-flight tool is not retroactively cancelled.

The registry is an application extension point, not a plugin marketplace.

### Permission rules

- Read-only local capabilities may use `Auto`, `Ask`, or `Off`.
- Write, execute, and network capabilities never offer `Auto`; their maximum permission is `Ask`.
- A tool must be enabled and its capability permission must allow the call.
- Approval is checked at call time.
- The first release ships no write, execute, or network tools.

### Initial Files / Read tools

The initial read-only pack auto-runs:

- `list_dir`: list at most 200 entries in a workspace directory, sorted by name, with a truncation marker.
- `read_file`: accepts a one-based `offset` (default 1) and `limit` in lines (default 200, maximum 1,000); output is capped at 24 KiB and returns the next offset when truncated.
- `find_files`: find paths using a glob; at most 200 results, 64 directory levels, 100,000 examined entries, and 24 KiB of output.
- `search_text`: regex search text files while skipping binary files and `.git`; at most 200 matches, 10 matches per file, 300 characters per displayed line, 100,000 examined entries, and 24 KiB of output.

All results have deterministic limits and explicit truncation markers. Invalid UTF-8, unreadable files, oversized input, and filesystem races return tool errors rather than panics.

### Workspace jail

Every tool path is resolved through a capability directory handle rooted at the attached workspace, using handle-relative/open-at semantics rather than a check-then-open absolute path. Absolute inputs, NULs, parent traversal, symlinks, Windows junctions/reparse points, and case-folded paths that escape the root are rejected. `find_files` and `search_text` never follow symlinks or junctions while walking.

Tools cannot inspect paths outside the attached workspace, even if a model supplies `..`, an absolute path, or an escaping symlink.

The first release hard-denies known sensitive paths: `.env`, `.env.*`, `*.pem`, `*.key`, `.ssh/**`, `.aws/**`, `.config/gcloud/**`, and files whose basename is `credentials`, `credentials.json`, `secrets`, or `secrets.json`. Denials are visible tool errors. There is no override until read approvals are designed.

### Compact and full controls

The agent context panel shows compact family rows with active counts and expandable tool toggles. `Manage tools` opens the full Agent Tools settings view, where family capabilities, permissions, defaults, future locked tools, and session overrides are visible.

## `/btw` side conversations

Typing `/btw` opens an ephemeral side thread using a frozen snapshot of completed conversation state at that moment.

- It never steers, pauses, or mutates the parent run.
- The parent agent never sees side-thread messages.
- It is multi-turn and has independent streaming state.
- In the first release it may use only local **Files / Read** tools, regardless of broader parent permissions.
- It cannot use write, execute, or network capabilities.
- This restriction is represented in the policy model so it can become configurable later.
- If Ollama is busy, it displays `Waiting for model`; the parent is not paused.
- `Keep as branch` promotes it to a branch with the parent snapshot plus side-thread transcript.

An incognito `/btw` remains memory-only.

## Branches

`Branch from here` is available on completed assistant responses and through `/branch` for the latest completed assistant response. A user message that has not received a completed assistant response is not a branch boundary.

A branch:

- copies transcript content through the selected completed message;
- copies model, model options, workspace, and the effective tool set at the fork point;
- records parent session and message identifiers;
- starts with no active run, no queued steering, and no in-flight tool;
- never replays copied tool calls;
- diverges independently and receives its own title; and
- appears once in history with a parent reference.

Partial, interrupted, failed, or cancelled assistant turns and all tool records are not valid branch points. Completed tool records before the selected assistant response are copied but never replayed. Later parent changes do not flow to the branch. Deleting a parent does not delete branches; their parent reference becomes `Source deleted`.

## History manager

`Manage` opens a dedicated History view that replaces the chat rail and conversation, so each session is listed exactly once. The list supports:

- search across titles and persisted message text;
- filters for All, Pinned, Chats, Agents, Branches, and live Incognito;
- Pinned and chronological groups;
- running status, model, workspace, message count, and branch-parent summaries;
- sort by recent activity;
- open, rename, pin/unpin, Auto-title checkbox, and permanent delete; and
- clear one or all live incognito sessions.

The right inspector describes only the selected list item; it does not repeat the list. Permanent deletion requires deliberate confirmation. Deleting an active session first stops its run. Incognito rows show memory-only status and offer Clear, never Save.

## Visual behavior

The Chat surface uses the approved history rail and no session tab strip. Agent tool steps, steering messages, interrupted turns, `/btw`, branch provenance, incognito warnings, and background-run indicators use the established compact card and status language.

All four theme families define semantic tokens rather than per-widget colors. Every screen remains usable at the existing minimum window size. Long titles, paths, model names, tool arguments, messages, and search results truncate or scroll without overlapping controls. Headless layout tests render every family in light and dark at the minimum window size, and palette tests require WCAG contrast of at least 4.5:1 for text/background and button-label/button-fill pairs.

The approved Tail mark is the vector in `docs/design/roost-tail.svg`; UI and packaging assets derive from that file rather than reinterpreting the comp.

Theme-family tokens are design data and use these exact values:

| Family | Mode | bg | surface | surface2 | line | fg | fg2 | fg3 | accent | accent-fg | accent-soft |
|---|---|---|---|---|---|---|---|---|---|---|---|
| Meadow | Light | `#EEF0F2` | `#FFFFFF` | `#F5F6F8` | `#DDE0E5` | `#16191D` | `#535A64` | `#6F7782` | `#0B6E66` | `#FFFFFF` | `#DFF1EE` |
| Meadow | Dark | `#101316` | `#171B1F` | `#1D2227` | `#2A3036` | `#E7EAEE` | `#A3ABB5` | `#7E8793` | `#3CC2B3` | `#06201D` | `#14312E` |
| Ember | Light | `#F3EFEA` | `#FFFFFF` | `#F8F4F0` | `#E3DAD1` | `#211813` | `#6B5A50` | `#7B675B` | `#C2410C` | `#FFFFFF` | `#FBE4D8` |
| Ember | Dark | `#15100D` | `#1F1814` | `#281F1A` | `#352821` | `#F4EDE6` | `#B8A69A` | `#938175` | `#FB923C` | `#2A0F03` | `#3A2418` |
| Plum | Light | `#F2EFF4` | `#FFFFFF` | `#F7F3F9` | `#DED7E4` | `#19131F` | `#63586B` | `#74657E` | `#7E22CE` | `#FFFFFF` | `#F0E3FA` |
| Plum | Dark | `#120E18` | `#1B1522` | `#241C2C` | `#30263B` | `#EFE9F5` | `#B3A6C0` | `#9183A0` | `#C084FC` | `#24112F` | `#2E1D3E` |
| Tide | Light | `#EDF1F4` | `#FFFFFF` | `#F4F7F9` | `#D8E0E7` | `#121A20` | `#52636F` | `#647783` | `#0369A1` | `#FFFFFF` | `#DCEEF8` |
| Tide | Dark | `#0D1216` | `#141B21` | `#1B252C` | `#26323B` | `#E6EDF3` | `#9AAAB7` | `#8194A1` | `#38BDF8` | `#08202D` | `#112D3D` |

All families share semantic status colors: light `ok #1B7F3B`, `ok-soft #E0F3E5`, `warn #9A6200`, `warn-soft #FBEFD3`, `bad #B3261E`, `bad-soft #FBE3E1`; dark `ok #5CC97B`, `ok-soft #15301C`, `warn #E5B04A`, `warn-soft #33270E`, `bad #F2766B`, `bad-soft #3A1D1B`.

## Rename and migration

Roost replaces hf2ollama in user-facing names, binary/package identity, window title, application paths, keychain service, environment variables, documentation, and release artifacts. The existing generic portable-mode marker remains the file `portable`.

Migration rules:

1. Prefer Roost locations and identifiers.
2. When Roost data is absent, detect legacy hf2ollama data and migrate it once using atomic copy/rename behavior appropriate to the platform.
3. Never overwrite newer Roost data with legacy data.
4. Preserve an unreadable or partially migrated legacy source and report a recoverable warning.
5. The Roost keychain service is read automatically. When it is empty, Settings offers an explicit **Import hf2ollama token** action and warns that macOS may show a Keychain access prompt. That action reads the legacy service, writes and verifies the Roost service, and leaves the legacy entry intact for downgrade compatibility.
6. Development, screenshot, and guarded E2E environment variables are renamed to `ROOST_*` without legacy aliases; no public hf2ollama release consumed them.
7. Portable mode continues to use the marker `portable`. In that mode, migrate `hf2ollama.toml` to `roost.toml`; `history.json`, `logs/`, and `staging/` already resolve to the same paths and must not be duplicated.
8. Existing imported Ollama models and import provenance remain intact.

Legacy logs and disposable default staging caches outside portable mode are left untouched rather than copied. A configured staging path is preserved through settings migration. Roost never deletes legacy application directories automatically.

As verified on 2026-10-27, the repository has merged pull requests and locally tested packaging, but no tags, GitHub releases, or release-workflow runs. Roost is the first public product release. Migration remains required to protect settings, import history, staging ownership, and credentials created by development builds.

## Architecture boundaries

- Core remains UI-free and owns Ollama HTTP, streaming decode, session persistence, migrations, agent state, tool policy, workspace enforcement, and tool execution.
- GUI owns egui state, rendering, interactions, and command/event folding.
- GUI code does not perform network calls or execute tools.
- Runtime commands are intent messages; runtime events are ordered state updates with stable session/run/turn identifiers.
- Cancellation is explicit and terminal events are idempotent.
- Persisted formats are versioned and migration-tested.

## Acceptance criteria

Roost is ready when:

1. Existing Browse, import, Installed, Settings, and job-dock tests remain green.
2. A user can create a saved chat, stream a response, restart Roost, and reopen the transcript.
3. A user can create an incognito chat or read-only agent and verify no session content is persisted after close or restart.
4. A user can run an agent with all four Files / Read tools and cannot escape the workspace through absolute paths, traversal, or symlinks.
5. A user can steer a streaming run and see the preserved interrupted turn followed by the applied steering message.
6. A `/btw` conversation proceeds without pausing or changing its parent and can be promoted to a branch.
7. A branch copies only completed history, inherits settings at the fork point, and diverges independently.
8. The history manager shows each session once and supports search, filters, pinning, Auto-title, rename, open, and delete.
9. Auto-title follows the event thresholds and ten-minute floor, waits for idle, and is disabled by manual rename.
10. A model without Ollama's `tools` capability remains usable for chat but cannot start an agent or host-action request.
11. Long transcripts retain their full local history while requests deterministically trim complete old message groups to the model context budget.
12. Unreachable, missing, slow-loading, and stalled models end a turn once with retry/reselection controls and no transcript loss.
13. Persisted sessions use per-session files plus a metadata index; updating one message never rewrites unrelated transcripts.
14. Sensitive-file patterns, symlinked walkers, Windows reparse points, traversal, and case-folded escapes cannot expose data through tools.
15. Existing hf2ollama settings, import history, portable data, and credentials migrate without data loss or breaking downgrade access to the legacy credential.
16. Meadow, Ember, Plum, and Tide pass contrast and minimum-size headless layout tests in light and dark variants.
17. Workspace tests, formatting, linting, and unit tests pass on the supported Rust toolchain.

## Delivery sequence

Implementation is split into independently reviewable stages:

1. Roost identity, migration, themes, and foundational session types.
2. Streaming chat, persistence, incognito, rail, and background session runtime.
3. Agent loop, tool registry, Files / Read tools, workspace jail, and steering.
4. `/btw`, branches, auto-title host action, and the dedicated history manager.

Each stage leaves the application buildable and testable. Later stages consume stable interfaces introduced by earlier stages rather than rewriting them.
