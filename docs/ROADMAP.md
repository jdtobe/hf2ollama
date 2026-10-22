# Roadmap

This roadmap is drawn from the design spec (`docs/superpowers/specs/2026-10-22-hf2ollama-design.md`): its scope, acceptance criteria, known risks and non-goals. It is a statement of intent, not a schedule. There are no release dates and no releases yet.

## Now

- Build the GUI crate: window shell, Browse, Installed, Settings, downloads dock and theme, matching `docs/design/ui-mockup.html`.
- Wire the GUI to the core through the command and event channels.
- Meet the v1 acceptance criteria in section 12 of the spec: search and metadata, GGUF import, supported safetensors import with a chosen quantization, unsupported repos detected before download, cleanup guarantees, non-default and remote Ollama, pause/resume/cancel/logs, delete from Installed, light and dark themes, one binary per platform with portable mode.

Already done in the core library: Hugging Face and Ollama clients, compatibility classification, fit estimation, hardware detection, settings and portable paths, staging and cleanup, history, import pipeline, job manager. See [CHANGELOG.md](../CHANGELOG.md).

## Next

Items from the spec's "known risks" that need verification or work before or around v0.1:

- Verify on Linux and Windows: Ollama safetensors `quantize` values (verified on macOS and Ollama 0.40.8 only) and general rendering (the UI spike was checked on macOS only).
- Verify the supported-architecture list with real test imports; only `LlamaForCausalLM` has been imported so far. The list is data and can be updated as Ollama's support changes.
- Verify `quantize` on a GGUF create and the error behavior for an unsupported architecture (both unverified).
- Narrow-window behavior: real wrapping of step chips and long names.
- Warn in the UI when Ollama reports a version below the minimum verified version (0.40.8).
- Release packaging: one self-contained binary per platform, plus combined third-party license notices.

## Later

- Cross-restart resume of downloads (not implemented in v1).
- VRAM detection beyond NVIDIA and Apple Silicon (for example AMD and Intel), subject to reliable detection.
- Accessibility improvements; egui has weaker accessibility than native toolkits.
- A command-line front end on top of `hf2ollama-core` (the library split leaves room for it; not planned in detail).
- Handling of orphan blobs if Ollama ever adds a blob-delete API (today they are documented, not worked around).

## Non-goals for v1

These are deliberately out of scope:

- Running or chatting with models.
- Editing Modelfiles beyond the model name.
- Converting architectures Ollama does not support (no Python or llama.cpp conversion step).
- Uploading models to Hugging Face.
- Managing Ollama's own library pulls.
