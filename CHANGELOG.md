# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project intends to adhere to [Semantic Versioning](https://semver.org/spec/v2.0.0.html) once it has a first release. There are no releases yet.

## [Unreleased]

Pre-release. The core library exists; the GUI is under construction.

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
- CI: format, clippy and test on macOS, Windows and Linux.
- Project documentation: README, contributing guide, code of conduct, security policy, architecture, FAQ, development and roadmap documents, and MIT OR Apache-2.0 licenses.

### Changed

- Spec and plan aligned with the verified Ollama safetensors `quantize` values and Hugging Face hash rules.
