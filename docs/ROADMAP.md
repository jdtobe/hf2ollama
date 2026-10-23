# Roadmap

This roadmap is drawn from the design spec (`docs/superpowers/specs/2026-10-22-hf2ollama-design.md`): its scope, acceptance criteria, known risks and non-goals. It is a statement of intent, not a schedule. There are no release dates and no releases yet.

## Done

- Core library: Hugging Face and Ollama clients, compatibility classification, fit estimation, hardware detection, settings and portable paths, staging and cleanup, history, import pipeline, job manager, model card fetch, token check.
- Desktop app: window shell, Browse (search, filters, details, model card, import plan), downloads dock, Installed, Settings, light and dark themes, startup sweep of orphaned job folders.
- Retry of the download step (3 times, backoff) after network errors, 5xx and 429; a confirmation when closing the window with running downloads; a name-collision re-check before `create` and protection against starting the same name twice; settings enum values written in kebab-case (old values still load); Clear cached files also removes failed downloads.
- Project files, CI (fmt, clippy, tests on three OSes, `dev-hooks` job) and a manual release workflow (automatic releases on merge to `main` are the planned next step; see the comment in `release.yml`) and cargo-deny.
- A real end-to-end run against a throwaway Ollama 0.40.8 on macOS; see [the acceptance record](superpowers/notes/acceptance.md).

See [CHANGELOG.md](../CHANGELOG.md).

## Now

- Finish the visual verification that needs an unlocked display and capture the README screenshots (listed in the acceptance record).
- Run the CI matrix for the first time on Linux and Windows and fix whatever it finds; verify the Linux package list.
- Release packaging: one self-contained binary per platform, plus combined third-party license notices.

## Next

Known limitations to address after the first release candidate:

- Resume of failed downloads: a Retry or Resume action for a failed job that reuses the kept partial files, including after a restart (today the files are kept for 7 days but a new import starts over).
- An overwrite prompt for a name that already exists (today the import is refused and overwrite is not offered from the GUI).
- The log viewer is cramped at the 900 by 640 minimum window (about two or three lines); a floating log viewer would fix it. Full logs are in `<log_dir>/job-<id>.log`.
- Ollama does not report free space or the model store path, so the Installed usage bar shows two segments (models imported with this app, and all others) and the status bar omits the store path.
- Each settings change creates a new job manager and keeps the old ones, so a long session leaks a few idle managers and the parallel limit is per manager. Prune idle managers.
- Pin the CI actions to full commit SHAs.
- The folder dialog (`rfd`) blocks the UI thread while open and returns nothing if no XDG portal is running on Linux.
- Screenshots in the README.
- Verify on Linux and Windows: Ollama safetensors `quantize` values (verified on macOS and Ollama 0.40.8 only; `int8` and no quantization were imported end to end, `int4` was not) and general rendering.
- Verify the supported-architecture list with more real test imports; only `LlamaForCausalLM` has been imported. The list is data and can be updated as Ollama's support changes.
- Verify `quantize` on a GGUF create and the error behavior for an unsupported architecture.
- Warn in the UI when Ollama reports a version below the minimum verified version (0.40.8).

## Later

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
