# Follow-ups after the first build

Everything below was found during review and deliberately left out of the first build. Items are grouped by when they should be done. The full history of findings and decisions is in `build-ledger.md`.

## Before a release

- Run CI on macOS, Windows and Linux for the first time. The workflows have only been read and parsed, never run. Verify the Linux apt package list for `eframe`/`winit`/`rfd` (xdg-portal), and pin the GitHub Actions to commit SHAs.
- Manual pass on Windows and Linux: keychain backends (`keyring`), the folder picker (`rfd`), a long Pause then Resume, file-lock behavior when deleting staging directories.
- Visual verification pass against `docs/design/ui-mockup.html` in both themes. No screenshots exist after the Browse view because the build machine's display was locked. Owed captures: dock at 900x640 with many rows and a log open, light 1180x820 finished and finished-with-log, every Installed, Settings and About capture, the quit dialog (including Cmd+Q on macOS), the enabled import button.
- Check `int4` and GGUF `quantize` imports, a remote Ollama, and the Ollama minimum version on a second machine.
- Enable GitHub private vulnerability reporting (see `SECURITY.md`) and add real screenshots to the README.

## First follow-up release

- **Second running instance.** The startup sweep treats any marked job directory without a state marker as an orphan, so a second instance on the same staging folder can delete the first one's in-progress download. Fix with a lock file per job directory (`fs4` is already a dependency).
- **One job manager.** Every settings or token change adds a `JobManager`, so `max_parallel` is per manager and can be exceeded briefly after a change, and queued jobs keep old settings. Replace with one manager and a swappable pipeline context.
- **Compatibility gaps.** Shard groups ignore the directory; incomplete shard sets and `mmproj-*.gguf` files are offered as models; extension matching is case-sensitive in places; weights in subfolders are not downloaded but the repo is treated as importable; duplicate `fp16` safetensors variants are downloaded. Require a root weight file, check shard completeness, lowercase extensions.
- **Free space.** Show the staging folder's free space in the status bar and the import card (available locally through `fs4`).
- **Resume after restart.** Partial downloads are kept for 7 days and can be cleared, but a new import starts fresh. Reuse a partial directory for the same repo and file.
- **Overwrite prompt.** The GUI always sends `overwrite: false` and refuses an existing name. Add an explicit "replace existing model" confirmation.
- **Log viewer.** At the 900x640 minimum window an open log shows about two lines. Move logs to a floating viewer or give an open log more room.
- **Folder dialog** blocks the UI thread while open.
- Prune old log files (`job-<id>.log` is never removed).
- Warn when the Hugging Face endpoint is `http://` (the token would travel in clear).
- Reject Windows reserved file names and trailing dots in downloaded paths.
- Pending import name: clear it if the backend never answers a start request (today it clears on `JobAccepted` only).
- Corrupt settings: if renaming the file to `.bak` fails, do not allow the next save to overwrite it; also the notice is written to `startup.log` twice.
- Quit dialog: it returns to idle without closing if every job settles while it is open.
- Partial failures in "Clear cached files" are reported in the log only, not in the UI.
- Tests: replace the wall-clock sleeps in the delete double-click tests and the quit tests; add a tooltip explaining why an armed Confirm button is briefly inert.

## Nice to have

- Settings: `http://a:b:c` passes URL validation; token field undo history is cleared on leaving the view but not zeroized in memory.
- Model card: UTF-8 BOM before front matter, ordered-list start numbers, first heading longer than 900 characters, tests for typical README sizes.
- The 1.5 s pause test in the e2e suite starts the job already paused; a long real-network pause is untested.
- Process notes (history cannot be changed): a few RED commits do not compile or fail clippy because of stubs; one JobControl implementation landed inside a test commit.
