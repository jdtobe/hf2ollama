# Acceptance record (spec section 12)

Date: 2026-10-22. Machine: macOS (Apple Silicon), Rust 1.99.1. Branch `feat/hf2ollama`.

Rule for this file: PASS only where something was actually run; everything else is NOT VERIFIED with the reason.

## Real Ollama run

- Server: a separate throwaway server started with `OLLAMA_HOST=127.0.0.1:11555 OLLAMA_MODELS=<temp dir>/models /Applications/Ollama.app/Contents/Resources/ollama serve`. `GET /api/version` returned `{"version":"0.40.8"}`. The temp models directory is a non-default `OLLAMA_MODELS` (the store the server logged was that temp path). The server was stopped afterwards by its own PID and the directory deleted.
- Safety guard (`e2e_environment()` in the test file, run by every test): the tests skip with a printed reason unless the URL variable is set, is http(s) on a loopback host (`HF2OLLAMA_E2E_ALLOW_REMOTE=1` allows another host), has a port other than 11434 (`HF2OLLAMA_E2E_CONFIRM_DISPOSABLE=1` allows it), and `GET /api/tags` shows the server is empty or holds only `hf2o-e2e-*` models (no override). Every delete goes through a wrapper that panics on any other name, and a drop guard deletes a test's models even when an assertion fails. Pure rules are unit-tested in the normal test run (`guard_tests`). Running a test against `http://127.0.0.1:11434` printed `SKIPPED: refusing ... port 11434 is Ollama's default` without sending a request. No request was ever sent to port 11434.
- The user's own Ollama was not running during the runs.
- Binary choice: the app bundle's `ollama` 0.40.8 is the version the API behavior was verified against (see `api-verification.md`). The Homebrew client at `/opt/homebrew/bin/ollama` is 0.40.5 and was not used, so there is no 0.40.5 behavior to report; the run matches the previously verified 0.40.8 behavior with no differences found.
- Hugging Face: public API only, no token. Models: `ggml-org/tiny-llamas` (`stories260K.gguf`, 1.2 MB; `stories15M.gguf`, 98 MB for the cancel test) and `hf-internal-testing/tiny-random-LlamaForCausalLM` (safetensors, 4 MB weights).
- Command: `HF2OLLAMA_E2E_OLLAMA_URL=http://127.0.0.1:11555 cargo test -p hf2ollama-core --test e2e_real_ollama -- --ignored --test-threads=1 --nocapture`

Verbatim result (re-run after the guard fix, fresh server on 11555 with a new temp `OLLAMA_MODELS`, version 0.40.8; the first run before the fix gave the same 8 passes):

```
running 8 tests
test cancel_in_the_middle_of_a_download_removes_the_job_dir ... cancel requested at Some((8192, 98357920)) bytes
ok
test gguf_import_deletes_staging_and_records_history ... ok
test gguf_import_with_delete_off_keeps_the_files_listed ... ok
test imports_work_against_a_non_default_models_directory ... throwaway Ollama 0.40.8 at http://127.0.0.1:11555
ok
test name_collision_errors_then_overwrite_succeeds ... ok
test pause_holds_a_job_until_resume ... ok
test safetensors_import_with_int8_and_unquantized ... hf2o-e2e-st-int8: /api/show details.format = "safetensors"
hf2o-e2e-st-none: /api/show details.format = "safetensors"
ok
test unsupported_repositories_are_classified_not_imported ... ok

test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 9 filtered out; finished in 11.94s
```

Forced failure (`HF2OLLAMA_E2E_FORCE_FAIL=1`, which makes the GGUF test fail after the model exists), then `/api/tags`:

```
test gguf_import_deletes_staging_and_records_history ... FAILED
thread 'gguf_import_deletes_staging_and_records_history' (8778684) panicked at crates/hf2ollama-core/tests/e2e_real_ollama.rs:456:5:
HF2OLLAMA_E2E_FORCE_FAIL=1: failing on purpose; the cleanup guard must still delete the model
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 16 filtered out; finished in 1.22s
{"models":[]}
```

The cleanup guard removed the model although the test failed; `/api/tags` was empty afterwards, and also empty after the full run.

What each test proves (file `crates/hf2ollama-core/tests/e2e_real_ollama.rs`):

| Test | Checks |
|---|---|
| `gguf_import_deletes_staging_and_records_history` | `run_import` ends `Finished`; model listed in `/api/tags`; history maps it to the repo; staging empty with delete-after-import on; `/api/show` format `gguf`; `OllamaClient::delete` removes it |
| `gguf_import_with_delete_off_keeps_the_files_listed` | with delete off, exactly one non-partial cached entry of at least the file size; `clear_cached` frees it |
| `safetensors_import_with_int8_and_unquantized` | both `quantize: Some("int8")` and none import, appear in `/api/tags`, `/api/show` reports `details.format = "safetensors"`, staging empty, delete works |
| `cancel_in_the_middle_of_a_download_removes_the_job_dir` | cancel requested at 8192 of 98,357,920 bytes: result `Cancelled`, `Cancelled` event, job dir gone (keep partials off), model not installed |
| `pause_holds_a_job_until_resume` | a job started paused does not finish or do any work for 1.5 s; after resume it finishes and the model exists |
| `name_collision_errors_then_overwrite_succeeds` | second import without overwrite fails with `NameCollision` and stages nothing; with overwrite it succeeds |
| `unsupported_repositories_are_classified_not_imported` | a GPT-2 repo (PyTorch weights only) is `Unsupported`; the Llama safetensors repo is `Safetensors`; the GGUF repo is `Gguf` (classification only) |
| `imports_work_against_a_non_default_models_directory` | an import works on the throwaway server whose `OLLAMA_MODELS` is a temp directory (all the other tests also ran against it) |

Limits of this run: pause and resume was tested through `JobControl` (job started paused, then resumed), not through the GUI buttons or `JobManager`; `int4` and other `quantize` values were not imported; GGUF `quantize` and a safetensors repo with an unsupported architecture that has safetensors weights were not imported; only macOS was used.

## Section 12 criteria

| Criterion | Result | Evidence |
|---|---|---|
| Search Hugging Face, filter, and view metadata for a result | PARTIAL: logic PASS, visual NOT VERIFIED | Core: `hf` search and `model_info` tests (offline, recorded fixtures); the e2e run called the live `model_info` for three repos. GUI: Browse state, filters, detail and layout tests at 900x640 and 1180x820 (`browse::*` tests). The live search UI was not driven by hand and no screenshots exist (display locked). |
| Import a GGUF file into Ollama and see it in Installed and in `ollama list` | PASS for the import and `/api/tags` (which `ollama list` reads); Installed view NOT VERIFIED live | `gguf_import_deletes_staging_and_records_history` (model in `/api/tags`). The Installed view is covered by state and layout tests with a fake Ollama only; it was not shown against the live server. `ollama list` itself was not run. |
| Import a supported safetensors repo with a chosen quantization | PASS for `int8` and none on 0.40.8 (macOS) | `safetensors_import_with_int8_and_unquantized`. `int4` not imported. |
| Unsupported repos are detected before download and explained | PASS (classification and wording); GUI callout NOT VERIFIED visually | `unsupported_repositories_are_classified_not_imported`; `compat` unit tests; Browse tests for the unsupported callout and disabled import. |
| No job leaves files in staging after success, cancel or failure (except failed-job files kept for 7 days when enabled); with delete off, kept files appear under Cached files | PASS for success, cancel and delete-off; failure covered by unit tests only | e2e: `gguf_import_deletes_staging...`, `cancel_in_the_middle...`, `gguf_import_with_delete_off...` (cached listing). Failure and partial handling: `pipeline` and `staging` unit tests (a failed job's files are kept for 7 days and are not reused by a new import; "Clear cached files" now removes them, tested in `clear_cached_removes_partial_dirs_but_never_active_or_foreign_ones`); `startup_sweeps_orphans_but_never_foreign_files`. Settings "Cached files" list: backend and state tests, not seen live. |
| Works against an Ollama with a non-default models directory and against a remote Ollama | Non-default directory PASS; remote Ollama NOT VERIFIED | The throwaway server used a temp `OLLAMA_MODELS`. A remote (other-machine) Ollama was not available; only a loopback server on another port was used. |
| Pause, resume, cancel, per-job logs, and dismissal behave as specified | PARTIAL | Core: pause/resume/cancel e2e above (via `JobControl`) and `JobManager` unit tests. GUI: dock reducer and layout tests (`dock::jobs`, `dock::layout_tests`) including dismiss and auto-dismiss. Resuming a failed job is not implemented (no action exists; partial files are not reused), and resuming after a restart is not supported. After a long Pause the connection may be stale: the download step now retries (unit tests with a dropped connection, `download_retries_after_a_dropped_connection_and_resumes_from_the_part_file`; not run against the live server). The dock was not exercised by hand against the live server; per-job log files are written (`job-<id>.log`) and unit-tested. |
| Delete from Installed removes the model through the API | PASS at API level; UI NOT VERIFIED live | `OllamaClient::delete` in every e2e test (model absent from `/api/tags` afterwards). The Installed delete button is covered by `installed` state tests and wiremock backend tests only. |
| Light and dark themes both pass a visual check against the mockup | NOT VERIFIED | The build machine's display was at the lock screen, so captures were blank (not bypassed). Headless layout tests exist for both themes. See the owed list below. |
| Single binary per platform, with a portable mode | PARTIAL | macOS release build succeeds: `target/release/hf2ollama-gui`, 22,410,144 bytes. Portable mode: `portable_marker_redirects_all_paths_beside_exe` and `startup_with_portable_marker_writes_nothing_outside_exe_dir` (nothing is created in the OS directories, files land beside the exe dir). Linux and Windows binaries were not built here (CI `release-build` job is written but has never run). |

## Other checks run for this task

- Startup: `startup_with_portable_marker_writes_nothing_outside_exe_dir`, `startup_without_marker_uses_os_dirs`, `startup_sweeps_orphans_but_never_foreign_files`, `unusable_saved_staging_folders_fall_back_with_a_notice`, `startup_notice_is_shown_in_the_status_bar_and_stays_inside_the_window`.
- No OpenSSL: `cargo tree -i openssl-sys` and `cargo tree -i native-tls` both report that no package matches. rustls is the TLS stack.
- `cargo deny check`: advisories ok, bans ok, licenses ok, sources ok (four documented "unmaintained" advisory ignores in `deny.toml`; 27 duplicate-version warnings are allowed by policy).
- The CI workflow YAML was parsed with Ruby's YAML loader only. GitHub Actions did not run, so the workflows themselves are untested.

## Visual verification still owed (needs an unlocked display)

1. Downloads dock at 900x640 with many rows (dark and light), and with one log open (dark and light).
2. Light theme at 1180 wide: dock with finished rows and finished-with-log.
3. Every Installed view capture (both themes, 900 and 1180): empty, loaded, filtered, expanded details, armed delete, toast, Ollama unreachable.
4. Every Settings view capture (both themes, 900 and 1180): all cards, token states, invalid inputs, two-step Clear, dev-hooks note.
5. The About (model card) tab captures: loading, loaded, truncated, no card, error.
6. A final side-by-side comparison of both themes against `docs/design/ui-mockup.html`.
7. README screenshots (a TODO comment marks the place).
8. A manual run on Linux and Windows (build, window, folder dialog, keychain).

## After the final review fixes

The retry, name re-check, quit confirmation and Clear cached changes were covered by unit and headless tests (wiremock, raw sockets, `MemoryStore`, temp directories). The ignored end-to-end tests against a throwaway Ollama were not re-run after these changes, and the quit dialog was not looked at on a real display.
