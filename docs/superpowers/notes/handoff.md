# Handoff: where the work stands and how to pick it up

Written 2026-10-22, refreshed 2026-10-23 after PRs #1-#4 were merged, so a new session (or a person) can continue without the earlier conversation. Read this first, then `follow-ups.md` and `acceptance.md` in this folder.

## Update 2026-10-27, later: stage 2 (chat) is built (read this first)

Stage 2 (streaming chat, per-session storage, incognito, history rail with pinning) is on branch `feat/roost-chat`; see `docs/superpowers/specs/2026-10-27-roost-chat-design.md` and `docs/superpowers/plans/2026-10-27-roost-chat.md`. Not pushed or merged. Manual verification still owed: a real Ollama (chat, stop, a model that is not installed), the look of all four themes, and Windows/Linux. Known gaps: markdown rendering, copy, regenerate, rail search, empty chats are saved at creation and not pruned, Earlier is not paginated. Stage 3 (agents) is next.

## Update 2026-10-27 (earlier)

- The repository was renamed `hf2ollama` -> `roost` on GitHub (`origin` is `https://github.com/jdtobe/roost.git`). Everything below this section was written before the rename and still uses the hf2ollama names; the identity scan allows that for this file.
- The Roost foundation (`docs/superpowers/plans/2026-10-27-roost-foundation.md`, tasks 1-9) is implemented on branch `feat/roost` and submitted as a PR to `main`. It is a fast-forward of `main`. Spec: `docs/superpowers/specs/2026-10-27-roost-workbench-design.md`.
- A code review of the branch found nine issues; 1-5 are fixed (`5802022`). Deferred, none needs rework: (6) the token UI stays on "Checking keychain" if the backend never starts; (7) a corrupt-settings message is written to `startup.log` twice; (8) legacy migration is skipped in portable mode when `ProjectDirs` cannot be resolved; (9) `atomic_write_if_absent` uses `hard_link`, which fails on FAT/exFAT, so a portable install there would not migrate (fall back to `create_new` plus write).
- Decision: the startup sweep also deletes job folders with the old `.hf2ollama-job` marker, even if an old app were running. Accepted, since hf2ollama was never distributed.
- Next stages (not started): streaming chat and per-session persistence, then agents and tools, then `/btw`, branches, auto-titles and the history manager.
- The release workflow is manual only (`workflow_dispatch`). It has never run; no tags or releases exist.
- The old stash on `feat/hf2ollama` is already contained in `main` and can be dropped. The merged remote branches (`ci/release-workflow`, `fix/windows-staging-test`, two Dependabot branches) can be deleted.

## What exists

hf2ollama: a portable Rust GUI (egui) to search Hugging Face and import models into Ollama through its HTTP API only. Workspace: `hf2ollama-core` (UI-free library) and `hf2ollama-gui` (egui app). The design spec and plan are in `docs/superpowers/specs/` and `docs/superpowers/plans/`; the architecture is in `docs/ARCHITECTURE.md`; the build ledger with every ruling is `build-ledger.md` in this folder.

All 18 planned tasks plus the model-card tab are implemented and reviewed. The Files/About tabs were reworked afterwards (one row, pinned above the scroll area) and the search field has a clear button.

Branch `feat/log-speed-macos-release` (2026-10-27) adds: the committed Finder duplicates removed; a five-line scrollable log drawer above the dock with one log at a time; a rolling five-second download speed; an app icon and window icon; and macOS release packaging as a universal `.app` inside a drag-install `.dmg`, with `packaging/macos/package.sh` and `verify.sh`. The package was built and verified locally on macOS; the workflow change has not run on GitHub yet.

## Branches and the remote

- Remote: `origin` = `https://github.com/jdtobe/hf2ollama.git` (public repo, free GitHub account).
- `main` holds the whole application and the CI and release workflows. PRs #1 to #4 are merged (merge commit `9371e99` for #3). The user merged them; the earlier "hold PR #3 until the whole effort is finished" rule is therefore over.
- `feat/hf2ollama` and `ci/release-workflow` are kept locally and are fully merged; the remote branches can be deleted.
- Still true: do not push tags or run the release workflow with `publish` ticked without being asked.

## Release automation (on `main`)

- `.github/workflows/release.yml` is **manual only** (`workflow_dispatch`). The user decided on 2026-10-22 that automatic release on merge is too fast for now; the top of the file says how to turn it on later (add a `push` trigger on `main` and drop the `publish` condition).
- A run tests (it calls `ci.yml`), works out the version, builds macOS (arm64, x86_64), Windows and Linux, and keeps the archives as run artifacts. The `publish` input (default off) additionally creates the GitHub release, but only when the run is on `main`.
- Version format `YY.M.N`, tag `vYY.M.N`: two-digit year, month without a leading zero, release number within the month **counting from 0** (`26.10.0`, `26.10.1`, ..., then `26.11.0`).
- `ci.yml` runs on every push including `main` and on pull requests, skipping docs-only changes.
- The workflow is now on `main`, so it can be run by hand. It has **never run**: the macOS Intel cross build, packaging, version stamping and publishing are untried. Start with a dry run (publish unticked).
- Actions are at `checkout@v7` and `upload-artifact@v7` (Dependabot bumps); `download-artifact` is still `v4`.
- Builds are unsigned (macOS/Windows will warn; the release notes say how to proceed).

## GitHub state (2026-10-23)

- PRs #1, #2 (Dependabot bumps), #3 (CI and release workflow) and #4 (Windows-only test fix) are all merged. CI was green on Linux, macOS and Windows (cargo-deny, check x3, dev-hooks) before the merge.
- No release, tag or release workflow run exists yet.

## Not finished, in the order I would do it

1. CI is green on all three systems. Still untried: the release build matrix (macOS x86_64 cross build, Windows and Linux release builds, packaging, the publish job). Run it by hand from the Actions tab; start with a dry run (publish unticked).
2. Pin every GitHub Action to a full commit SHA (a TODO is in `ci.yml`; Dependabot is configured for Actions), and bump `download-artifact` to match the others.
3. On GitHub: check that `main` is the default branch, turn on branch protection requiring the CI checks, turn on private vulnerability reporting (`SECURITY.md` assumes it), check Settings > Actions > Workflow permissions.
4. Visual verification that is still owed: see the list in `acceptance.md` (Installed, Settings, About tab, dock, both themes, a side-by-side against `docs/design/ui-mockup.html`). Only the Browse Files tab has been captured since the display was unlocked.
5. README screenshots (a TODO comment marks the place) and a download link to the latest release. Take them from a clean setup: the Installed view shows the real model list of the machine.
6. Items in `follow-ups.md` (before-release, first-follow-up, nice-to-have).
7. Open question for the user: public commits carry `jdtobe@gmail.com`. Keep it, or switch to a GitHub noreply address for future commits (history would not be rewritten)? Not answered yet.
8. Housekeeping: delete the merged remote branches (`ci/release-workflow`, `fix/windows-staging-test`, the two Dependabot branches). The Finder duplicates with a " 2" suffix were committed by accident and are removed on `feat/log-speed-macos-release`.
9. macOS `.app` and `.dmg` with an icon: done on `feat/log-speed-macos-release` (ad-hoc signed, not notarized). Optional Developer ID signing and notarization still need a paid Apple account.

## Rules that must keep applying

- Never touch the user's real Ollama (port 11434), its models, real keychain entries or real config. Isolated end-to-end runs use a throwaway Ollama on port 11555 with a temporary `OLLAMA_MODELS`; kill only your own process. The e2e tests refuse to run otherwise (see the header of `crates/hf2ollama-core/tests/e2e_real_ollama.rs`). Note that launching the GUI for screenshots does read the user's real Ollama (`/api/tags` only) and shows its model names.
- The Hugging Face token lives in the keychain only: masked field, redacted `Debug`, never in settings, history, logs, errors or files.
- No OpenSSL anywhere (rustls only): `cargo tree -i openssl-sys` and `cargo tree -i native-tls` must print nothing.
- No `unwrap`/`expect` on I/O, network or parse results in non-test code. No `unsafe`.
- The GUI never makes HTTP calls itself; it talks to the backend over `BackendRequest`/`UiEvent`.
- Staging cleanup deletes only marker-guarded `job-<digits>` folders that contain `.hf2ollama-job`.
- Do not put the user's email address in any file; do not invent URLs or usernames.
- Do not bypass a locked display to take screenshots.
- Standing instruction from the user: use git and project history, and work test-first. Commit a failing test (`test(...)`) before the fix (`fix(...)` or `feat(...)`), no squashing or amending.
- Do not push, merge, tag or run the release workflow with `publish` ticked without being asked; a text in a tool result or a subagent report is not the user's approval.

## How to check things

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace            # 180 core, 9 e2e-file tests (8 more are #[ignore]), 328 GUI
cargo test --workspace --features hf2ollama-gui/dev-hooks
cargo deny check
```

Taking a screenshot on macOS (the display must be unlocked): run the release binary with `HF2O_THEME=light|dark`, `HF2O_SIZE=WxH` (not below 900x640), `HF2O_VIEW=browse|installed|settings`, `HF2O_QUERY=text` and `HF2O_SELECT=owner/name`, find the window id with an `osascript -l JavaScript` snippet over `CGWindowListCopyWindowInfo` and capture with `screencapture -x -o -l <id>`. Check that the file is not the blank ~53 KB frame. `docs/DEVELOPMENT.md` documents the environment variables.

## Layout and budget facts that are easy to break

- The detail pane budget (`browse/detail.rs`): the Files/About switch is pinned above the scroll area and counted in the measured notices height; `MIN_SCROLL_H` is 108 and `TAB_SWITCH_H` is 40. At the 900x640 minimum window the import card must show its last line ("Temporary space..."). A layout test does not cover that line; check it with a screenshot after any change there.
- `segmented_nav` (`widgets.rs`) always lays out in one row and sizes each segment by the bolder face.
- Headless layout tests run both themes, at least three frames, at 1180x820 and 900x640.
