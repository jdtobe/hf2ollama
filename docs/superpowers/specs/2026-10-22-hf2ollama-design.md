# hf2ollama Design Spec

Date: 2026-10-22
Status: Draft for review

## 1. Purpose

A portable, professional-looking Rust desktop app that lets a user browse and search Hugging Face, pick the exact model file or quantization they want, and have it installed in Ollama. It also lets them see and manage what is installed.

Success looks like this:

- A user finds a model, sees the metadata needed to choose (size, quantization, architecture, context, license, whether it fits their machine), and imports it in a few clicks.
- GGUF repos work end to end. Safetensors repos work when Ollama supports the architecture. Unsupported repos are detected before download and explained.
- Nothing is left behind: temporary files are removed according to the user's setting.
- The app works with any Ollama the user can reach, including one whose models directory is not `~/.ollama/models`.
- It ships as a single self-contained binary per platform (macOS, Windows, Linux).
- It is pleasant for developers to work on: small crates, clear boundaries, tests that run without a network.

Non-goals for v1: running or chatting with models, editing Modelfiles beyond the model name, converting architectures Ollama does not support (no Python or llama.cpp conversion step), uploading models to Hugging Face, managing Ollama's own library pulls.

## 2. Decisions already made

| Decision | Choice | Why |
|---|---|---|
| Ollama integration | HTTP API only | Server owns its store, so any location or remote host works. Survives store-format changes. |
| GUI toolkit | egui / eframe | Single portable binary, permissive license (MIT/Apache), fastest to build. A spike with a design pass reached close to the mockup. Slint was the alternative (GPLv3 or royalty-free license terms). Tauri was rejected because it needs a system webview and is not a fully self-contained binary. |
| Architecture | Core library + thin GUI binary | UI-free logic is testable and leaves room for a CLI. |
| Cleanup default | Delete downloaded files after import | User request. Configurable. |
| Look and feel | Matches `docs/design/ui-mockup.html` | Approved over several UI rounds. |

## 3. Architecture

Cargo workspace with two crates.

```
hf2ollama/
  crates/
    hf2ollama-core/    library, no UI dependencies
    hf2ollama-gui/     binary, egui/eframe
  docs/
    design/ui-mockup.html
    superpowers/specs/
```

### 3.1 `hf2ollama-core` modules

| Module | Responsibility | Depends on |
|---|---|---|
| `hf` | Search models, fetch model metadata and file list (with sizes and SHA-256), download files with HTTP Range resume. | `reqwest`, `tokio` |
| `ollama` | Client for `/api/version`, `/api/tags`, `/api/show`, `/api/blobs/:digest` (HEAD, POST), `/api/create`, `/api/delete`. Streams progress. | `reqwest`, `tokio` |
| `pipeline` | Runs one import job: plan, download, verify, upload, create, cleanup. Emits events. Cancellable and pausable. | `hf`, `ollama`, `staging` |
| `staging` | Per-job folders, disk-space checks, cleanup guarantees, startup sweep of orphans and expired partials. | std |
| `fit` | Pure function: model size and machine profile to a fit verdict. | none |
| `hardware` | Detects RAM and VRAM where reliable. Accepts a manual override. | `sysinfo`, platform APIs |
| `compat` | Decides whether a repo is importable: GGUF, safetensors with a supported architecture, or unsupported (with a reason). | none |
| `config` | Settings load and save (TOML). Portable-mode detection. | `serde`, `toml`, `directories` |
| `secrets` | Hugging Face token in the system keychain. | `keyring` |
| `history` | Local record of imports (Hugging Face repo per Ollama model name), stored as JSON. | `serde_json` |
| `events` | Event and command types shared with the GUI. | none |

Rules for the crate:

- Public API is async and returns typed errors (`thiserror`). No `unwrap` on I/O.
- TLS uses rustls, so there is no OpenSSL dependency.
- Core never imports egui. The GUI never makes HTTP calls directly.

### 3.2 `hf2ollama-gui`

- Owns a tokio runtime. UI sends commands to core over a channel and receives events over another. The UI thread never blocks on I/O.
- The event receiver calls `ctx.request_repaint()` so progress updates draw immediately.
- Views: Browse, Installed, Settings, plus the downloads dock and status bar. Each view is its own module with its own state struct.
- A `theme` module holds the design tokens, font setup and custom widgets (tag, fit pill, quality bar, segmented nav, primary button, table row).

## 4. Import pipeline

One job per import. States: `Queued → Downloading → Verifying → Uploading → Creating → CleaningUp → Done`, plus `Paused`, `Failed(reason)`, `Cancelled`.

1. **Plan.** Resolve the selected file or files. Compute download size and final size. Check free space in the staging folder. Check Ollama is reachable. Refuse with a clear message if space is insufficient.
2. **Download** into `<staging>/<job-id>/`. Resume with HTTP Range. Hash with SHA-256 while streaming. Use the Hugging Face token for gated repos.
3. **Verify** each file against the SHA-256 published by Hugging Face for LFS files. Small non-LFS files (config, tokenizer) publish only a git blob SHA-1, so verify that instead. The app also computes the SHA-256 of every file locally, because Ollama's blob upload needs it. A mismatch fails the job and deletes the bad file.
4. **Upload** each file to Ollama as a blob (`HEAD /api/blobs/:digest` to skip if present, else `POST`). The digest is the file's SHA-256.
5. **Create** the model with `POST /api/create`, mapping file names to blob digests. For safetensors the request includes every needed file (weights, config, tokenizer files) and a `quantize` value chosen by the user.
6. **Clean up** per settings (section 6). A successful job records an entry in `history`.

Controls:

- **Pause** stops reading and keeps the partial file. **Resume** continues from the partial file.
- **Cancel** stops the job and deletes the partial files.
- A failed job keeps resumable partial downloads for 7 days if that setting is on. Otherwise it deletes them.
- Each job has a timestamped log (queued, request results, stage completions, pause, resume, cancel, errors). The log stays available until the job row is dismissed. It is also written to a log file for bug reports.

## 5. Compatibility rules

`compat` classifies a repo from its file list and metadata:

| Repo contents | Result |
|---|---|
| One or more `.gguf` files | Importable. Show each file as an option. Split GGUF shards are grouped and treated as one option. |
| `*.safetensors` plus `config.json` and tokenizer files, architecture in the supported list | Importable. User picks a quantization from the values Ollama accepts for safetensors (verified on Ollama 0.40.8: none, `int8`, `int4`; kept as bundled data, see `docs/superpowers/notes/api-verification.md`). |
| Safetensors with an unsupported architecture | Not importable. Explain why. |
| PyTorch `.bin`, ONNX, or other formats only | Not importable. Explain why. |

For non-importable repos the UI shows related GGUF repos when search finds them, and disables the import button.

The supported-architecture list is data, not code: a small file bundled with the app that can be updated as Ollama's support changes.

## 6. Settings and storage

| Setting | Default | Notes |
|---|---|---|
| Ollama address | `http://localhost:11434` | Has a Test button. Models directory is shown read-only as reported by the server. |
| Staging folder | OS cache directory | User can change it. Needs room for one model at a time. |
| Delete downloaded files after import | On | Off keeps GGUF and safetensors files in staging. Listed under Cached files with a Clear button. |
| Keep partial downloads for 7 days | On | Cancelling always deletes. |
| Simultaneous downloads | 2 | Range 1 to 3. |
| Auto-dismiss finished and cancelled jobs | Never | Options: never, 1 minute, 5 minutes, 1 hour. |
| Hugging Face token | none | Stored in the system keychain. |
| Hugging Face endpoint | `https://huggingface.co` | For mirrors. |
| Hardware override | none | For when Ollama runs on another machine. |
| Theme | Match system | Light, dark, or system. |

Storage locations:

- Settings: TOML in the OS config directory.
- Import history: JSON beside the settings.
- Logs: OS log or state directory.
- **Portable mode:** if a file named `portable` sits next to the executable, settings, history and logs are stored beside it, and the default staging folder is beside it too.

## 7. UI

The reference is `docs/design/ui-mockup.html`. Everything below is required behavior, and the mockup is the visual source of truth.

### 7.1 Window shell

- Top bar: app name, segmented nav (Browse, Installed, Settings, with an installed count), search field (Browse only), Ollama connection pill with version and address.
- Bottom: downloads dock, then a status bar showing staging folder, free space, and the Ollama store path reported by the server.

### 7.2 Browse

- Left filter rail: format (GGUF, safetensors, PyTorch or other), parameter range, "only models that fit", task, sort.
- Results list: owner and name, format tag, parameters, architecture, downloads, likes, last updated. Selected row has an accent background and left stripe.
- Detail pane:
  - Header with name, author, downloads, last updated, and an "Open on huggingface.co" link.
  - Metadata strip: parameters, architecture, context length, license, base model, prompt template.
  - For safetensors repos, a banner explaining full-precision download and Ollama-side quantization.
  - Table of files or quantizations: name, size, estimated memory need, quality bar and note, and an "On your machine" fit pill. The recommended row is labeled.
  - Import plan card: editable Ollama model name, pipeline steps, temporary space, final size, staging free space, and the **Download & import** button.
  - For non-importable repos, an explanation and related repos instead of the import card.

### 7.3 Installed

- Data from `/api/tags` and `/api/show`.
- Table: model, parameters, quantization, size, source, modified. Source is a link to the Hugging Face repo for models in `history`, otherwise "Ollama library".
- Disk usage bar split into imported by this app, pulled from Ollama, and free space.
- Text filter, source filter, sort, refresh.
- Row details: family, context, capabilities, digest, run command.
- Delete uses a two-click confirmation ("Confirm delete", which expires after 4 seconds).

### 7.4 Settings

Sections for Ollama, Downloads, Cached files, Hugging Face, and Hardware and appearance, containing the settings in section 6. Changes apply immediately.

### 7.5 Downloads dock

- One row per job: model name, size, progress bar, stage and percent, Pause or Resume, Cancel, Log.
- Finished and cancelled rows stay until dismissed. They show a Dismiss button and keep their log. A "Clear finished" button appears when any exist.
- Header count: "N active", "All done", or "No active jobs".

### 7.6 Design system

Light and dark palettes use the same tokens as the mockup:

| Token | Light | Dark |
|---|---|---|
| bg | `#eef0f2` | `#101316` |
| surface | `#ffffff` | `#171b1f` |
| surface-2 | `#f5f6f8` | `#1d2227` |
| line | `#dde0e5` | `#2a3036` |
| fg | `#16191d` | `#e7eaee` |
| fg-2 | `#535a64` | `#a3abb5` |
| fg-3 | `#8a919b` | `#6f7782` |
| accent | `#0b6e66` | `#3cc2b3` |
| accent-fg | `#ffffff` | `#06201d` |
| accent-soft | `#dff1ee` | `#14312e` |
| ok / ok-soft | `#1b7f3b` / `#e0f3e5` | `#5cc97b` / `#15301c` |
| warn / warn-soft | `#9a6200` / `#fbefd3` | `#e5b04a` / `#33270e` |

- Fonts: Hanken Grotesk (regular and semibold) for text, JetBrains Mono (regular and medium) for data. Bundled in the binary. Both are OFL-licensed, so the license text ships with the app.
- Egui's variable-font limitation means static weights must be generated and embedded, not the variable files.
- Custom-drawn components: tag, fit pill, quality bar, segmented nav, primary and small buttons, table rows, selected-row stripe, progress bars.
- Follow the system theme by default, with an explicit light or dark override in Settings.

## 8. Fit estimation

`fit(size_gb, machine) -> Fits | Tight | PartialOffload | TooLarge`. Memory need is file size plus an overhead allowance for context. Thresholds are constants in one place and covered by tests. RAM comes from `sysinfo`. VRAM is detected where reliable (NVIDIA, Apple unified memory) and otherwise left unknown, in which case the UI says so and offers the manual override. Treat results as estimates, not guarantees.

## 9. Error handling

Errors are shown in plain language with the next step, and every failure has a log entry.

- Ollama unreachable: pill turns red, Browse still works, import button explains the problem and offers Test and Settings.
- Insufficient disk space: refuse before downloading, state needed and available space.
- Gated or private repo: prompt for a token in Settings.
- Network failure: job fails with partial file retained (per setting) and a Resume action.
- Hash mismatch: delete the file, fail the job, say the download was corrupt.
- Ollama rejects `create` (for example, unsupported architecture): show Ollama's message, clean up per settings.
- Name collision with an existing model: ask before overwriting.

## 10. Testing

- `core` unit tests: `fit`, `compat`, `staging` (including cleanup on success, failure and cancel, and orphan sweep), `config` and portable-mode detection.
- `core` integration tests against mock HTTP servers with recorded Hugging Face and Ollama responses: search and metadata parsing, resume after interruption, hash mismatch, blob already present, create failure, full GGUF and safetensors pipelines, pause, resume, cancel.
- GUI: widget and layout logic kept thin. Screenshot checks of the main views in light and dark at the default window size and a narrow size, run manually for v1.
- CI builds on macOS, Windows and Linux.

## 11. Known risks and items to verify before implementation

1. **VRAM detection** differs by vendor. Mitigation: reliable cases only, plus manual override.
2. **Ollama API shape.** Confirm the current `/api/create` request fields for file maps and `quantize`, blob upload semantics, and progress streaming against the Ollama version in use. Record the minimum supported Ollama version.
3. **Hugging Face API.** Confirm which endpoint returns per-file SHA-256 and GGUF metadata (architecture, context length) and the rate limits for unauthenticated use.
4. **Supported architectures.** Build the initial supported list from Ollama's documented safetensors support, and verify with test imports.
5. **egui limits.** Weaker accessibility than native toolkits. Narrow-window behavior needs real wrapping of step chips and long names. Both are tracked as UI requirements.
6. **Windows and Linux rendering.** The spike was checked on macOS only.
7. **Orphan blobs in Ollama.** Ollama has no blob-delete API, so a failed import after upload can leave an unused blob in its store (observed: 4 MB). Ollama prunes unreferenced blobs on restart, so this is documented in the UI and README, not worked around.
8. **Safetensors `quantize` values were verified on macOS only** (Ollama 0.40.8: `int4`, `int8`, runner `mlx`). Linux and Windows behavior is unverified. The quantization list is bundled data, and an Ollama rejection is shown as-is.

## 12. Acceptance criteria for v1

- Search Hugging Face, filter, and view metadata for a result.
- Import a GGUF file into Ollama and see it in Installed and in `ollama list`.
- Import a supported safetensors repo with a chosen quantization.
- Unsupported repos are detected before download and explained.
- With the delete setting on, no job leaves files in staging after success, cancel or failure (other than resumable partials when enabled). With it off, kept files appear under Cached files.
- Works against an Ollama with a non-default models directory and against a remote Ollama.
- Pause, resume, cancel, per-job logs, and dismissal behave as specified.
- Delete from Installed removes the model through the API.
- Light and dark themes both pass a visual check against the mockup.
- Single binary per platform, with a portable mode.
