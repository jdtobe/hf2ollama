# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project intends to adhere to [Semantic Versioning](https://semver.org/spec/v2.0.0.html) once it has a first release. There are no releases yet.

## [Unreleased]

Pre-release. The core library and the desktop app (Browse, model card, downloads dock, Installed, Settings) are implemented and tested; nothing is released or packaged yet.

### Added

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
- CI: format, clippy and test on macOS, Windows and Linux; a `dev-hooks` clippy and test job; a `release-build` job (manual run or `v*` tag) that builds the GUI binary per OS and uploads it as an artifact.
- Project documentation: README, contributing guide, code of conduct, security policy, architecture, FAQ, development and roadmap documents, and MIT OR Apache-2.0 licenses.
- Desktop app: window shell with a top bar, view navigation, Ollama status pill and status bar showing the staging folder; light and dark themes with the bundled fonts; minimum window size 900 by 640.
- Browse view: search with filters, a result list, lazily loaded repository details, GGUF and safetensors option tables with fit pills, a quantization picker (none, int8, int4 with estimated sizes), default model name rules, and a pinned import plan card. Gated, unsupported and unreachable states are explained, and failed loads have a Retry.
- Model card ("About this model"): the repository README is fetched by core (size-capped, front matter stripped) and drawn as a safe subset of Markdown with parse and draw budgets (no raw HTML, no images, only absolute http(s) links).
- Downloads dock: one row per job with progress, step chips, pause, resume, cancel, dismiss, per-job log, auto-dismiss from Settings, and a height cap so Browse keeps its minimum room.
- Installed view: models from the Ollama API with the Hugging Face source recorded by this app, a two-segment usage bar (imported with this app, other), sorting and filtering, details, and delete with two-step confirmation.
- Settings view: Ollama address with a connection test, staging folder (read-only check while typing, write probe on commit), delete-after-import, keep partials, parallel downloads, auto-dismiss, Hugging Face endpoint and token (system keychain, checked against Hugging Face), hardware override, theme, and cached files with a two-step Clear. Changes apply live and are saved to the TOML file.
- Core: `HfClient::whoami` (token check with a readable invalid-token error and redaction) and `HfClient::model_card`.
- Startup: orphaned and expired job folders in the staging folder are swept once off the UI thread (only marker-guarded job folders; the report goes to `startup.log`).
- Opt-in end-to-end tests against a real Ollama and the public Hugging Face API (`crates/hf2ollama-core/tests/e2e_real_ollama.rs`), and the acceptance record in `docs/superpowers/notes/acceptance.md`.
- Workspace package metadata: `license = "MIT OR Apache-2.0"`, authors, `rust-version = 1.95`, `publish = false`.
- `cargo-deny` now passes (licenses, bans, advisories, sources) with four documented "unmaintained" advisory ignores.
- Screenshot and trial environment variables: `HF2O_THEME`, `HF2O_SIZE`, `HF2O_VIEW`, `HF2O_QUERY`, `HF2O_SELECT`; the in-memory token store (`HF2O_MEMORY_TOKEN_STORE`, `HF2O_MEMORY_TOKEN`) exists only with the `dev-hooks` cargo feature.

### Changed

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
