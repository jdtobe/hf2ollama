# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project intends to adhere to [Semantic Versioning](https://semver.org/spec/v2.0.0.html) once it has a first release. There are no releases yet.

## [Unreleased]

Pre-release. The core library and the desktop app (Browse, model card, downloads dock, Installed, Settings) are implemented and tested; nothing is published yet. The macOS release is packaged as a universal app inside a DMG.

### Roost foundation

- The project is renamed Roost: crates `roost-core` and `roost-gui`, binary `roost`, settings `roost.toml`, keychain service `roost`, environment variables `ROOST_*` (the `HF2O_*` names are no longer read).
- One-way, non-destructive migration of hf2ollama settings and import history; legacy staging markers are still recognized; the Hugging Face token is imported only when the user asks in Settings.
- Metadata writes (settings, history) are atomic and collision-safe.
- Four theme families (Meadow, Ember, Plum, Tide), each light and dark, chosen in Settings; the old `theme` setting still loads. Palettes meet 4.5:1 text contrast and are layout-tested at the minimum window size.
- The Roost mark and icon, generated from `docs/design/roost-tail.svg`, and a CI scan for stray old names.
- Serializable chat and agent session types in `roost-core`. Chat and agents themselves are not implemented yet.

### Chat and history (stage 2)

- A Chat view, now the first tab and the starting view: streaming replies from any installed Ollama model, a history rail (Pinned, Today, Yesterday, Earlier), New chat and New incognito, rename, pin and two-step delete, a model picker, Retry for failed, stopped or interrupted replies, and a Stop button while a reply runs.
- Conversations are saved one file per session with a rebuildable index. Damaged files are set aside, files from a newer Roost are left untouched, and a crash during a reply keeps the partial text (marked Interrupted). Only one Roost can use a history at a time.
- Incognito chats exist only in memory: nothing is written to disk, and their drafts and editor state are dropped when they end.
- Requests send an explicit `num_ctx` (the model's context length, capped at 16,384) and trim old exchanges to fit, keeping the full transcript locally.
- Replies continue in other sessions and while you use Browse, Installed or Settings; the Chat tab shows a count of running replies.
- Fixed: the top bar's logo, tabs, search field and Ollama status pill now share one vertical centre.

### Chat refinements

- A delete control on every rail row: hover or select a row, click the ✕, then click "Delete?" after a short pause.
- Loaded models: the chat header shows whether its model is in memory (and when Ollama will unload it) with an Unload button, and the rail has an "In memory" list with Unload for each loaded model. Ollama is asked every 10 seconds and whenever a reply starts or ends.

- Chats without a title show the start of their first message in the rail and the header (display only; a rename always wins).

### One Models tab

- **Models** is one view instead of Browse and Installed behind a switch: a list with the models Ollama has (with a usage bar) above the Hugging Face results, one search box that narrows both, and one detail pane. Filters are chips above the list (a Filters popover, Fits my machine, and the sort order) instead of a side rail.
- Selecting an installed model shows **Chat**, **Work** and **Delete…** (two clicks, with the usual pause), then its details and where it came from; the source repository is a link that opens it in the same list.
- The list: installed rows are taller, with the parameter count, quantization and size as pills; the disk-usage bar is a titled card with a legend; the sort order is a full-width dropdown with a filter button at its right end (the button keeps its place and shows how many filters are on; its popover holds every filter, including Only models that fit, and a Reset button). Hugging Face results default to GGUF only.
- Model pages are three stacked bands in the theme's own colours. An installed model: the header with Chat, Work and a red trash can (tinted with the accent), the details, then the description. A Hugging Face repository: the header with any warnings, the files to choose from, then the description (no more Files/About tabs). The import panel at the bottom appears only after you pick a file; nothing is picked for you any more (the recommended file is still marked).
- A Hugging Face result for a repository you have imported from shows an **installed** tag. The installed model's detail ends with **About**: the source repository's description (or a note that models pulled from the Ollama library have none).
- The View menu has one Models entry (⌘3); ⌘4 and the separate Browse and Installed entries are gone.

### A quieter window

Five rounds of design (mockups and evaluations in `docs/design/ui-declutter-mockups.html`); the busiest screen went from about 25 visible controls to about 13.

- **Tabs:** Chat, Work and Models, plus a settings gear. Browse and Installed are the two views inside Models (a small switch next to the tabs). The menu shortcuts still reach each of them.
- **Title row:** the title, a folder chip for agents, and one “···” menu holding Rename, Pin and Delete (delete asks for a second click). Incognito is a small badge instead of a banner.
- **Folder chip (Work):** the folder, step budget and tools are one popover instead of a second bar. It locks while a run is active.
- **Composer:** the model chip sits under the input and shows whether the model is in memory; its popover changes the model and unloads it. One status line above the input shows an agent's step (“Step 3 of 20”), messages waiting for its next step, and notices.
- **Tool steps** of an agent fold into one line (“Used 3 tools: search_text ×2, read_file”) with live “running…” and failure counts; open it for the cards.
- **Sidebar:** one “+ New” menu replaces the two buttons; the “In memory” footer moved into the Ollama status pill's popover, which lists loaded models with Unload.

### Fixes and polish after first use

- **+ New** is a split button: the main half starts a chat (or agent), the arrow offers the incognito variant.
- Shift+Enter adds a line in the message box; Enter sends.
- On narrow windows the status pill shows only the Ollama version (the address is in its hover text), so the Browse search field keeps its room.
- The import plan card shows its steps as one line (`Download › Verify › Upload › Create`) that never wraps.
- A search field in the Chat and Work sidebars (matches the title, first message and model).
- Assistant replies are drawn as Markdown (bold, lists, code blocks, tables, safe links) and every message has a Copy button.
- **Message actions:** a row of icons under a message (always on the latest, on hover for older ones): Copy, Edit (your messages), Regenerate (latest finished reply) and Details (model, tokens in and out, speed and time, for replies finished from now on).
- **Versions:** Regenerate and Edit keep what they replace. A message with other versions shows `‹ 2 / 3 ›` at the bottom-right of its row; the arrows switch between them (when no reply is running). Edit opens your message in place and sends it as a new version. Saved chats store the other versions; older files load unchanged.
- **Branch:** the branch icon on a finished reply starts a new chat with the history up to that reply, on the same or another model. A branched chat shows `↳ Branched from …` at the top (each name opens that chat; long chains collapse) and says whether the original is unchanged, changed since, or deleted; a divider marks where the copied history ends. Branches are listed under their parent in the sidebar.
- **Side questions (BTW):** the speech-bubble icon on a reply opens a popover where you ask something about the conversation up to that reply. It is answered separately (it works while a reply or agent run is going), is never added to the chat unless you press Add to chat, and is remembered per chat (newest first, up to 50, kept in the chat file; incognito chats keep them in memory). Earlier ones are listed under the answer, each with Add to chat and Delete.
- **Read aloud:** the speaker icon on a finished reply reads it with the computer's own voice (it becomes a Stop button while reading). macOS uses `say`, Windows uses PowerShell's speech synthesizer, and Linux uses `spd-say`, `espeak-ng` or `espeak` when one is installed; otherwise the icon is greyed with the reason. Markdown marks and code blocks are not read. No speech library is added.
- Fixed: the **+ New agent…** button did nothing while an agent chat was open. The New agent form now replaces the chat on screen.
- **Settings ▸ Ollama ▸ Keep models in memory:** Ollama's default, 15 minutes, 1 hour, or until Ollama quits. Sent as `keep_alive` with chat requests only when you choose something other than the default.
- **Downloads panel:** each job is two lines. The name (cut in the middle so the tag stays visible), size, a stage pill and the buttons; then a progress bar of the same width for every job with percent, bytes, speed and an estimate of the time left. The estimate uses a smoothed speed, waits for five seconds of data, is rounded, and does not jump about. The log drawer gained a Copy button.

### Agents (Work tab)

- A **Work** tab (between Chat and Browse) for read-only agents. An agent works inside a folder you choose and can list, find, search and read files there; it cannot change anything. Start one with New agent: pick a folder and a model that supports tools (Roost checks with Ollama first).
- The agent's steps are shown as tool cards (name, arguments, status, expandable result) between its replies. Tool failures are given back to the model as results.
- **Steering:** send a message while an agent works. It waits for the next safe step; a reply that is streaming is interrupted and kept (marked Interrupted) and the same run continues. Stop ends the run. A step budget (default 20, 1 to 100) ends a run that keeps calling tools.
- The agent's folder, step budget and tools can be changed while it is idle (Tools menu; the folder button). Incognito agents are kept in memory only.
- The workspace jail: tool paths cannot leave the folder through absolute paths, `..`, NUL, or symlinks, and `.env`, `.env.*`, `*.pem`, `*.key`, `.ssh`, `.aws`, `.config/gcloud` and files named `credentials*` or `secrets*` are never readable.
- Menu: View ▸ Work (⌘2; Browse and Installed move to ⌘3 and ⌘4) and File ▸ New Agent….

### Menu bar

- A real menu bar: native on macOS (app menu with About, Settings…, Quit; File, Edit, View, Models, Window, Help) and Windows, and an in-window bar on Linux. File: New Chat and New Incognito Chat. View: Chat, Browse, Installed, Settings and a Theme submenu (four colour families and System/Light/Dark). Models: Unload for each loaded model, or Unload All. Edit: Cut, Copy, Paste, Select All. Help: GitHub and issue links. An About window shows the version.
- Standard shortcuts (⌘N, ⇧⌘N, ⌘, , ⌘1–3, ⌘Q on macOS; the same with Ctrl on Windows and Linux). Quit still goes through the close confirmation.
- On Linux the window's minimum height grows by the bar so the content keeps its tested minimum size.

### Added

- A clear (✕) button in the Browse search field.
- Workspace with two crates: `hf2ollama-core` (library, no UI dependencies) and `hf2ollama-gui` (egui binary).
- Core library: shared event and command types, with a channel roundtrip test.
- Core library: Hugging Face search and model metadata (per-file sizes and SHA-256 for LFS files, GGUF metadata), with the access token redacted from `Debug` output.
- Core library: resumable, verified downloads using HTTP Range, with a check that a `206` response matches the resume offset, bounded header waits and cancellation that races network waits.
- Core library: Ollama API client (version, tags, show, blob HEAD and upload, create with streamed progress, delete).
- Core library: repo compatibility classification (GGUF, safetensors with a supported architecture, unsupported with a reason) and a bundled, data-driven list of supported architectures.
- Core library: fit estimation of a model against a machine profile.
- Core library: hardware detection (RAM, NVIDIA VRAM, Apple Silicon unified memory) with a manual override.
- Core library: settings and portable-mode paths (a `portable` file next to the executable keeps settings, history and logs beside it).
- Core library: staging folders with guaranteed cleanup, startup sweep of orphaned and expired partial folders, and listing and clearing of cached files.
- Core library: import history and Hugging Face token storage in the system keychain.
- Core library: Ollama model name validation.
- Core library: import pipeline (plan, download, verify, upload, create, cleanup) with per-job logs, history records, cancel and pause support, and guaranteed job directory cleanup.
- Core library: job manager with a parallel-download limit, queue, and pause, resume and cancel controls. A create that has already completed wins over a late cancel.
- Recorded Hugging Face and Ollama fixtures for offline tests, and notes on the verified API behavior in `docs/superpowers/notes/api-verification.md`.
- Bundled fonts (Hanken Grotesk, JetBrains Mono) with their SIL Open Font License texts.
- CI: format, clippy and test on macOS, Windows and Linux; a `dev-hooks` clippy and test job; a manual release workflow that builds a universal macOS app and DMG, plus Windows and Linux archives, and uploads them as artifacts.
- Project documentation: README, contributing guide, code of conduct, security policy, architecture, FAQ, development and roadmap documents, and MIT OR Apache-2.0 licenses.
- Desktop app: window shell with a top bar, view navigation, Ollama status pill and status bar showing the staging folder; light and dark themes with the bundled fonts; minimum window size 900 by 640.
- Browse view: search with filters, a result list, lazily loaded repository details, GGUF and safetensors option tables with fit pills, a quantization picker (none, int8, int4 with estimated sizes), default model name rules, and a pinned import plan card. Gated, unsupported and unreachable states are explained, and failed loads have a Retry.
- Model card ("About this model"): the repository README is fetched by core (size-capped, front matter stripped) and drawn as a safe subset of Markdown with parse and draw budgets (no raw HTML, no images, only absolute http(s) links).
- Downloads dock: one row per job with progress, a rolling five-second download speed, step chips, pause, resume, cancel, dismiss, per-job log, auto-dismiss from Settings, and a height cap so Browse keeps its minimum room. Clicking Log opens a five-line, scrollable drawer above the dock; one log is open at a time.
- App icon: a bundled teal download mark, used for the window and (on macOS) the app bundle and DMG volume.
- macOS release packaging: a universal `hf2ollama.app` for Apple Silicon and Intel inside a drag-install `hf2ollama-<version>-macos-universal.dmg`, ad-hoc signed and checked by `packaging/macos/verify.sh` in CI.
- Installed view: models from the Ollama API with the Hugging Face source recorded by this app, a two-segment usage bar (imported with this app, other), sorting and filtering, details, and delete with two-step confirmation.
- Settings view: Ollama address with a connection test, staging folder (read-only check while typing, write probe on commit), delete-after-import, keep partials, parallel downloads, auto-dismiss, Hugging Face endpoint and token (system keychain, checked against Hugging Face), hardware override, theme, and cached files with a two-step Clear. Changes apply live and are saved to the TOML file.
- Core: `HfClient::whoami` (token check with a readable invalid-token error and redaction) and `HfClient::model_card`.
- Startup: orphaned and expired job folders in the staging folder are swept once off the UI thread (only marker-guarded job folders; the report goes to `startup.log`).
- Opt-in end-to-end tests against a real Ollama and the public Hugging Face API (`crates/hf2ollama-core/tests/e2e_real_ollama.rs`), and the acceptance record in `docs/superpowers/notes/acceptance.md`.
- Workspace package metadata: `license = "MIT OR Apache-2.0"`, authors, `rust-version = 1.95`, `publish = false`.
- `cargo-deny` now passes (licenses, bans, advisories, sources) with four documented "unmaintained" advisory ignores.
- Screenshot and trial environment variables: `HF2O_THEME`, `HF2O_SIZE`, `HF2O_VIEW`, `HF2O_QUERY`, `HF2O_SELECT`; the in-memory token store (`HF2O_MEMORY_TOKEN_STORE`, `HF2O_MEMORY_TOKEN`) exists only with the `dev-hooks` cargo feature.

- Downloads: the download step retries up to 3 times (1 s, 2 s, 4 s; a retry-after hint is honoured up to 30 s) after a network error, a stall, a 5xx or a 429, continuing from the partial file; never after a checksum mismatch, a gated repo, other 4xx errors or a cancel. The wait honours cancel and pause, and each retry is logged.
- Closing the window with running downloads shows a confirmation ("Keep running" or "Quit and cancel"); quitting cancels every job and waits at most 3 seconds.
- The model name is checked again right before `create`, the Download button is disabled while another job uses the same name (and right after a click, so a double click starts one job), and a name collision fails with a readable message. Overwriting is still not offered.
- A download that sends more bytes than its declared size is stopped immediately.
- A job task that panics or is stopped now writes a log line and still ends with one terminal event.
- An unreadable settings file is renamed to `hf2ollama.toml.bak` before defaults are used (logged in `startup.log` and shown in the status bar).
- The typed Hugging Face token and its undo history are wiped when leaving Settings and after Save or Clear.
- Sweeping and clearing the staging folder continue past a folder that cannot be removed and report it.

### Fixed

- The Files and About tabs were stacked on top of each other and scrolled away with the page. They are now one row, pinned above the scrolling area, and no longer change width when selected.

### Changed

- The macOS release artifact is now one universal `.dmg` with an `.app` inside instead of two bare-binary `.tar.gz` archives.
- Settings enum values are written in kebab-case (`never`, `one-minute`, `five-minutes`, `one-hour`; `system`, `light`, `dark`). The earlier PascalCase spellings still load.
- Clear cached files in Settings now also removes the files of failed downloads, and the button is enabled whenever anything is listed. The Settings text and the docs no longer promise resuming: failed downloads are kept for 7 days and then removed, and resuming a failed job or after a restart is not supported yet.
- File sizes from the remote are summed with saturating arithmetic.
- Spec and plan aligned with the verified Ollama safetensors `quantize` values and Hugging Face hash rules.
- CI runs on pushes to `main` and on pull requests only (no duplicate runs), and no longer installs `libgtk-3-dev`, which nothing uses.
- Delete in Installed needs a 600 ms pause between arming and confirming, and held keys never confirm.
- The file dialog for the staging folder uses `rfd` with the XDG portal backend on Linux.
- A saved staging folder that is not an absolute path (or contains `..` or starts with `~`) is ignored at startup in favor of the default, with a notice in the status bar.
- Removed the unused `quality_bar` and `fit_pill` widgets.

### Security

- The in-memory token store is compiled only with the `dev-hooks` feature, needs the exact value `1`, prints a warning and is flagged in Settings, so a release binary cannot be switched to it.
- The Hugging Face token lives in the system keychain only, is passed through a redacting type and never appears in `Debug` output; Hugging Face cross-host redirects do not carry it (tested).
- Staging folder changes require an absolute path without `..` or `~`, and the write probe runs on commit with a unique file.
- Model card rendering is bounded (input size, nesting, blocks, spans, table cells, list items) so a hostile README cannot freeze the UI.
