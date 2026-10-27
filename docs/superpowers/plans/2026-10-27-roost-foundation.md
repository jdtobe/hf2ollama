# Roost Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rename the product safely to Roost, migrate existing hf2ollama metadata and credentials without data loss, ship the approved identity/theme families, and establish stable session-domain types for later chat and agent stages.

**Architecture:** Keep the two-crate boundary while renaming it to `roost-core` and `roost-gui`; the GUI binary is `roost`. Core owns paths, atomic persistence, migration, credentials, settings, and serializable session types. GUI owns the Roost mark, semantic themes, shell branding, and settings controls.

**Tech Stack:** Rust 2021 (MSRV 1.95), tokio, reqwest with rustls, serde/TOML/JSON, keyring, uuid, eframe/egui 0.37, wiremock and tempfile for tests.

**Spec:** `docs/superpowers/specs/2026-10-27-roost-workbench-design.md`

## Global Constraints

- `roost-core` remains UI-free. GUI code never performs HTTP calls, credential migration, or filesystem migration directly.
- Ollama is accessed only through its HTTP API; never read or write its model store.
- Do not start or contact the user's Ollama, real keychain, or real Roost/hf2ollama directories in tests. Use mocks, in-memory stores, and temporary directories.
- Keep reqwest on rustls; do not add OpenSSL.
- Preserve existing Browse, import, Installed, Settings, dock, and guarded end-to-end behavior.
- Product identifiers are `Roost` (display/app), `roost` (binary/data/keychain service), `roost-core`, and `roost-gui`. Legacy identifiers remain only in migration code/tests, historical records, and explicit migration documentation.
- Portable mode continues to use the marker file `portable`.
- New portable settings use `roost.toml`; existing `history.json`, `logs/`, and `staging/` remain in place.
- New OS data uses `directories::ProjectDirs::from("", "", "roost")`; legacy detection uses `hf2ollama`.
- New staging jobs carry `.roost-job`; cleanup recognizes both `.roost-job` and `.hf2ollama-job` and never deletes foreign files, directories, or symlinks.
- Development and guarded E2E variables are renamed to `ROOST_*` without legacy aliases.
- Every task ends with `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --features roost-gui/dev-hooks -- -D warnings`, and `cargo test --workspace --features roost-gui/dev-hooks` passing before commit.

## Review Focus

1. **Both Roost and legacy files exist:** Roost data wins and is never overwritten. Task 3 tests this for settings and import history.
2. **Migration fails partway through:** the source remains intact, an incomplete destination is not exposed, and startup returns a readable warning. Tasks 2 and 3.
3. **Credential promotion fails or prompts on macOS:** migration is explicit, the legacy credential remains, and secret text never appears in errors/debug output. Task 5.
4. **Mixed old/new staging markers plus hostile symlinks:** owned jobs are swept, foreign and symlinked paths survive. Task 4.
5. **All eight theme variants at minimum size:** controls remain inside the viewport and text/action pairs meet contrast requirements. Task 7.

---

### Task 1: Rename crates, packages, and binary

**Files:**
- Rename: `crates/hf2ollama-core/` → `crates/roost-core/`
- Rename: `crates/hf2ollama-gui/` → `crates/roost-gui/`
- Modify: `Cargo.toml`, `Cargo.lock`, both renamed crate manifests
- Modify: all Rust imports under both renamed crates
- Modify: `.github/workflows/ci.yml`

**Interfaces:**
- Produces package `roost-core`, Rust crate path `roost_core`, package `roost-gui`, and binary `roost` via `[[bin]] name = "roost", path = "src/main.rs"`.
- Preserves all public core APIs except the Rust crate prefix changes from `hf2ollama_core` to `roost_core`.

- [ ] **Step 1: Record the failing identity query.** Run `cargo metadata --format-version 1 --no-deps` and confirm it reports `hf2ollama-core`, `hf2ollama-gui`, and no `roost` binary.
- [ ] **Step 2: Rename the directories and manifests.** Use the descriptions from this plan's Architecture; add the explicit `roost` binary target.
- [ ] **Step 3: Replace Rust import paths and CI selectors.** Do not rename legacy migration values or historical documentation.
- [ ] **Step 4: Verify metadata** reports only packages `roost-core` and `roost-gui`, target `roost`, and GUI dependency path `../roost-core`.
- [ ] **Step 5: Run the Global Constraints verification commands and commit** `chore: rename workspace crates and binary to roost`.

---

### Task 2: Shared atomic persistence primitive

**Files:**
- Create: `crates/roost-core/src/persistence.rs`
- Modify: `crates/roost-core/src/lib.rs`, `config.rs`, `history.rs`

**Interfaces:**
- Produces `persistence::atomic_write(path: &Path, bytes: &[u8]) -> Result<()>` using a unique same-directory temp name, `File::sync_all`, atomic rename, and best-effort temp cleanup.
- Existing `Settings::save` and import `History::save` use this function without changing their public APIs.

- [ ] **Step 1: Write failing persistence tests:** nested parent creation, complete round-trip, a forced write/rename failure leaves an existing destination unchanged, failed writes leave no temp file, and 16 concurrent writers produce one fully parseable result without temp-name collision.
- [ ] **Step 2: Run** `cargo test -p roost-core persistence::`; expect compile failure.
- [ ] **Step 3: Implement `atomic_write`** with unique names from process ID plus an atomic counter. Sync the temp file before rename; do not use a fixed `.tmp` suffix.
- [ ] **Step 4: Replace the duplicate save implementations** in Settings and import History; retain their serialization formats and existing corrupt-file behavior.
- [ ] **Step 5: Run focused and Global Constraints verification; commit** `fix(core): make metadata writes collision-safe`.

---

### Task 3: Roost paths and legacy-file migration

**Files:**
- Create: `crates/roost-core/src/migration.rs`
- Modify: `crates/roost-core/src/lib.rs`, `paths.rs`
- Modify: `crates/roost-gui/src/startup.rs`, `main.rs`, `runtime.rs`

**Interfaces:**
- Produces `paths::Paths { config_file, import_history_file, sessions_dir, log_dir, default_staging, portable }`.
- Produces `paths::LegacyPaths { config_file, import_history_file, log_dir, default_staging, portable }`.
- `OsDirs::detect() -> OsDirs` resolves Roost; `OsDirs::detect_legacy() -> Option<OsDirs>` resolves hf2ollama.
- `Paths::resolve(exe_dir, os)` and `LegacyPaths::resolve(exe_dir, os)` remain pure and injectable.
- Produces `MigrationReport { migrated: Vec<&'static str>, warnings: Vec<String> }` and `migrate_legacy_files(current: &Paths, legacy: &LegacyPaths) -> MigrationReport`.
- `Startup` gains `migration: MigrationReport`; `startup(exe_dir, current_os, legacy_os)` migrates before settings load.

- [ ] **Step 1: Write failing path tests:** Roost OS paths use `roost.toml` and `sessions/`; portable mode keeps `portable`; portable legacy/current history, logs, and staging are identical while config names differ; legacy OS paths use hf2ollama directories.
- [ ] **Step 2: Write failing migration tests:** missing Roost files copy legacy settings/import history, existing Roost files win, same portable history is skipped, failed copy preserves source and destination state, and a second run is idempotent.
- [ ] **Step 3: Implement path resolution and migration** through `persistence::atomic_write`. Leave legacy sources, OS logs, and disposable OS staging caches untouched. A migrated configured staging path remains unchanged in TOML.
- [ ] **Step 4: Integrate startup notices.** Tests cover success, unreadable-source warnings, current-data precedence, portable confinement, and `sessions_dir` under the selected mode.
- [ ] **Step 5: Update runtime fixtures/callers** from `history_file` to `import_history_file` without changing provenance behavior.
- [ ] **Step 6: Run focused and Global Constraints verification; commit** `feat(core): migrate legacy metadata safely to roost`.

---

### Task 4: Staging ownership migration

**Files:**
- Modify: `crates/roost-core/src/staging.rs`
- Modify: `crates/roost-gui/src/settings/validate.rs`

**Interfaces:**
- New jobs write `.roost-job`.
- Sweep/list/clear recognize `.roost-job` and legacy `.hf2ollama-job` as ownership markers.
- Settings writable probes use `.roost-write-test-<pid>-<counter>`.

- [ ] **Step 1: Write failing tests:** new jobs use only the Roost marker; legacy-marker jobs remain manageable; mixed markers are counted correctly; symlinked job directories, unmarked lookalikes, and foreign files always survive.
- [ ] **Step 2: Implement dual-marker recognition** without changing the existing job-name guard or symlink refusal.
- [ ] **Step 3: Rename the writable-probe prefix** and retain exact cleanup-on-success/failure behavior.
- [ ] **Step 4: Run focused and Global Constraints verification; commit** `feat(core): recognize legacy staging ownership`.

---

### Task 5: Explicit credential migration and environment rename

**Files:**
- Modify: `crates/roost-core/src/secrets.rs`
- Modify: `crates/roost-gui/src/main.rs`, `runtime.rs`, `app.rs`, `settings/mod.rs`, `settings/state.rs`
- Modify: `crates/roost-core/tests/e2e_real_ollama.rs`

**Interfaces:**
- Produces `KeychainStore::roost()` using service `roost`, account `huggingface-token`, and `KeychainStore::legacy()` using service `hf2ollama`.
- Produces `promote_legacy_token(current: &dyn TokenStore, legacy: &dyn TokenStore) -> Result<LegacyTokenMigration>` where `LegacyTokenMigration::{Imported, NotFound}`; it writes and verifies current but never clears legacy.
- Adds `BackendRequest::ImportLegacyToken` and a result `UiEvent`; Settings shows the action only when no Roost token is stored.
- Produces pure environment parsers that accept an injected lookup closure; exact `"1"` is the only true flag value.

- [ ] **Step 1: Write failing credential tests:** current credential wins; successful promotion writes and verifies current but keeps legacy; failed write/verification keeps legacy and current state valid; errors/debug never contain token text.
- [ ] **Step 2: Implement explicit promotion.** Do not inspect legacy keychain at app startup. The Settings action says macOS may display a Keychain prompt before dispatching `ImportLegacyToken`.
- [ ] **Step 3: Integrate backend and Settings states:** Imported, NotFound, and readable non-secret Error; dev-hooks continue to inject MemoryStore and never expose the legacy-import action.
- [ ] **Step 4: Write failing environment tests** without process-global mutation, then rename screenshot/dev hooks to `ROOST_THEME/SIZE/QUERY/SELECT/VIEW/MEMORY_TOKEN_STORE/MEMORY_TOKEN` with no aliases.
- [ ] **Step 5: Rename guarded E2E variables** to `ROOST_E2E_*`; preserve loopback/default-port/model-prefix safeguards. Keep model prefix `hf2o-e2e-` for cleanup compatibility.
- [ ] **Step 6: Run focused and Global Constraints verification; commit** `feat(core): add explicit legacy credential import`.

---

### Task 6: Roost shell identity, provenance, and release packaging

**Files:**
- Create: `crates/roost-gui/src/identity.rs`
- Derive from: `docs/design/roost-tail.svg`
- Replace: `crates/roost-gui/assets/icon-256.png`, `icon-1024.png`
- Modify: `crates/roost-gui/src/main.rs`, `app.rs`, `installed/mod.rs`, `quit.rs`, `runtime.rs`, `startup.rs`
- Modify: `packaging/macos/package.sh`, `Info.plist.in`, `verify.sh`, `make_icon.py`
- Modify: `.github/workflows/release.yml`
- Create: `scripts/check_roost_identity.py`

**Interfaces:**
- Produces `identity::paint_tail(ui, size)` from the canonical SVG coordinates and colors.
- macOS product is `Roost.app`, executable `roost`, bundle ID `io.github.jdtobe.roost`, DMG `roost-<version>-macos-universal.dmg`.
- Windows/Linux archives are `roost-<version>-<matrix-name>` and contain `roost`/`roost.exe`.

- [ ] **Step 1: Write identity/layout tests:** app display constants are Roost values; the Tail painter stays in its allocated square at 16, 22, and 64 px.
- [ ] **Step 2: Implement the Tail painter and deterministic icon generation** from `docs/design/roost-tail.svg`; regenerate both committed PNGs.
- [ ] **Step 3: Replace user-visible copy and process/thread prefixes** with Roost: window title, brand, quit dialog, import labels, stderr prefix, backend/sweep/event thread names.
- [ ] **Step 4: Add a provenance regression test:** an existing `history.json` mapping still yields `source_repo` and the renamed `Imported with Roost` label. The label is presentation only; provenance lookup remains `History::source_of`.
- [ ] **Step 5: Update packaging/release scripts** and verifier assertions for app, executable, bundle ID, volume/archive names, and notes. Run `bash -n` on shell scripts.
- [ ] **Step 6: Implement the identity scan** with explicit allowances for migration code/tests and historical 2026-10-22 records. Wire it into `.github/workflows/ci.yml` on every platform.
- [ ] **Step 7: Run focused and Global Constraints verification; commit** `feat(gui): apply roost identity and packaging`.

---

### Task 7: Theme families and appearance settings

**Files:**
- Modify: `crates/roost-core/src/config.rs`
- Create: `crates/roost-gui/src/theme_tokens.rs`
- Modify: `crates/roost-gui/src/theme.rs`, `settings/mod.rs`, `settings/state.rs`, `app.rs`, `main.rs`

**Interfaces:**
- Renames `ThemeChoice` to `ThemeMode { System, Light, Dark }`; the Settings field `theme_mode` accepts old TOML field alias `theme` and old enum spellings.
- Produces `ThemeFamily { Meadow, Ember, Plum, Tide }`, default Meadow.
- `theme_tokens.rs` contains the exact design table from the spec.
- Produces `palette(family, dark)`, preserves `tokens(ui)` for existing views, and adds `apply(ctx, family, mode)`; selected family is stored in egui context data.

- [ ] **Step 1: Write failing config tests:** defaults Meadow/System; old `theme = "dark"` loads; all family/mode values round-trip in kebab-case; partial old files default Meadow.
- [ ] **Step 2: Write failing palette tests:** assert the spec table; WCAG contrast is at least 4.5:1 for `fg/surface`, `fg2/surface`, `fg3/surface`, and `accent-fg/accent` for all eight palettes.
- [ ] **Step 3: Implement config migration and token selection.** Theme changes apply immediately and save through the existing request path.
- [ ] **Step 4: Add Appearance controls** for family and System/Light/Dark mode. Headless tests cycle all eight concrete variants at the 900×640 minimum and assert controls/content remain inside their regions after scrolling.
- [ ] **Step 5: Update `ROOST_THEME`** to accept `meadow-light`, `meadow-dark`, `ember-light`, `ember-dark`, `plum-light`, `plum-dark`, `tide-light`, `tide-dark`.
- [ ] **Step 6: Run focused and Global Constraints verification; commit** `feat(gui): add accessible roost theme families`.

---

### Task 8: Stable session domain foundation

**Files:**
- Create: `crates/roost-core/src/session.rs`
- Modify: `crates/roost-core/src/lib.rs`, `Cargo.toml`

**Interfaces:**
- Adds `uuid = { version = "1.28", features = ["v4", "serde"] }` and `SESSION_SCHEMA_VERSION: u32 = 1`.
- Produces ID newtypes `SessionId`, `MessageId`, `RunId`, `TurnId` with UUID construction, display/parse, serde, ordering, and hashing.
- Produces `SessionMode { Chat, Agent }`, `SessionLifetime { Persisted, Incognito }`, `SystemPromptKind { Chat, Agent }`, `UserDelivery`, `AssistantStatus`, `ToolStatus`, and `RunState { Idle, WaitingForModel, Running, Stopping }` carrying stable IDs where active.
- Produces `ModelOptions { num_ctx: Option<u32>, temperature: Option<f32>, seed: Option<i64>, think: Option<bool> }`, default all unset.
- Produces `TitleMetadata { auto_title: bool, last_titled_at_ms: Option<u64>, undo_title: Option<String> }`.
- Produces registry-neutral `EffectiveToolConfig { revision: u32, capabilities: BTreeMap<String, Permission>, tools: BTreeMap<String, bool> }` and `Permission { Auto, Ask, Off }`; concrete registries consume this later.
- Produces `BranchOrigin`, transcript message/content types (User, Assistant, Tool), and `Session { id, mode, lifetime, title, title_metadata, pinned, model, model_options, system_prompt, workspace, effective_tools, step_budget, run_state, parent, created_at_ms, updated_at_ms, messages }`.
- Produces `SessionDocument { schema_version: u32, session: Session }`; `SessionDocument::new(session)` sets `SESSION_SCHEMA_VERSION` and `into_current()` rejects unknown future versions.
- Default `step_budget` is 20; chats default `SystemPromptKind::Chat`, agents default `Agent`.
- Produces constructors, `is_branch`, `is_idle`, `completed_prefix_through`, and manual rename. Valid branch boundaries are Complete Assistant messages only. Manual rename trims, rejects empty or over 80 Unicode-scalar characters, records undo, and disables Auto-title.

- [ ] **Step 1: Write failing ID/constructor tests:** IDs round-trip; chat/agent defaults include every field above; agent workspace must be absolute with no lexical `..`; lifetime is independent of mode.
- [ ] **Step 2: Write failing serialization tests:** a fully populated session round-trips with schema version, model options, tool snapshot, title metadata, run IDs, Unicode transcript, and tool JSON.
- [ ] **Step 3: Write failing state tests:** `is_idle` is false for active run states, streaming assistants, and pending/running tools; loaded active states can be normalized to Idle with the affected assistant marked Interrupted for crash recovery.
- [ ] **Step 4: Write failing branch tests:** only Complete Assistant boundaries succeed; User-only, Tool, Streaming, Interrupted, Failed, and Cancelled boundaries fail; prefix preserves copied IDs/content and excludes later messages.
- [ ] **Step 5: Write failing title tests:** rename trims, records undo, disables Auto-title, updates time, rejects invalid titles without mutation.
- [ ] **Step 6: Implement only domain types and validation.** Per-session persistence is Stage 2; do not add HTTP or agent execution.
- [ ] **Step 7: Run focused and Global Constraints verification; commit** `feat(core): add stable session domain types`.

---

### Task 9: Contributor documentation and foundation verification

**Files:**
- Modify: `README.md`, `CHANGELOG.md`, `CONTRIBUTING.md`, `SECURITY.md`, `THIRD_PARTY_LICENSES.md`
- Modify: `docs/ARCHITECTURE.md`, `docs/DEVELOPMENT.md`, `docs/FAQ.md`, `docs/ROADMAP.md`
- Modify: `.github/ISSUE_TEMPLATE/bug_report.yml`
- Preserve as historical: 2026-10-22 specs/plans/notes

**Interfaces:**
- Documents current package commands, `ROOST_*` variables, explicit credential import, migration behavior, per-session storage direction, four delivery stages, and unchanged real-Ollama safety guards.

- [ ] **Step 1: Update current user/contributor docs.** Clearly mark chat/agents as staged until implemented; do not claim planned functionality is shipped.
- [ ] **Step 2: Document verified release state:** merged PRs and local packaging exist, but as of 2026-10-27 there are no tags, GitHub releases, or release-workflow runs.
- [ ] **Step 3: Run the CI-wired identity scan** and inspect each allowed old-name occurrence; remove accidental allowances.
- [ ] **Step 4: Run full foundation verification:** Global Constraints commands, `python3 scripts/check_roost_identity.py`, `bash -n packaging/macos/package.sh packaging/macos/verify.sh`, and `git diff --check`.
- [ ] **Step 5: Confirm no generated archives, secrets, user config, or target artifacts are staged; commit** `docs: update project guidance for roost foundation`.
