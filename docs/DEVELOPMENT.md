# Development

## Setup

- Stable Rust with `rustfmt` and `clippy` (`rustup component add rustfmt clippy`).
- Linux only: `sudo apt-get install -y libxkbcommon-dev libwayland-dev libx11-dev libgl1-mesa-dev` (the packages CI installs).
- An Ollama server is only needed for manual end-to-end checks. Tests do not need one or a network.

```sh
cargo build
```

## Repo tour

```
Cargo.toml                    workspace (resolver 2, edition 2021)
crates/
  hf2ollama-core/             library, no UI dependencies
    src/                      modules described in docs/ARCHITECTURE.md
    src/supported_architectures.json   bundled data
    tests/fixtures/           recorded Hugging Face and Ollama responses
  hf2ollama-gui/              egui/eframe binary
    assets/                   bundled fonts and their OFL license texts
docs/
  ARCHITECTURE.md FAQ.md DEVELOPMENT.md ROADMAP.md
  design/ui-mockup.html       approved visual reference
  design/egui-spike-reference.rs   egui spike used as a style reference
  superpowers/specs/          the design spec (source of truth)
  superpowers/plans/          the implementation plan
  superpowers/notes/          api-verification.md, acceptance.md
.github/                      CI, cargo-deny workflow, templates, dependabot
deny.toml                     cargo-deny policy
```

## Running the checks

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

CI also runs clippy and the tests with `--features hf2ollama-gui/dev-hooks`.

All three must pass before each commit. Run a single crate or test while iterating:

```sh
cargo test -p hf2ollama-core
cargo test -p hf2ollama-core fit
```

Optional, if you have it installed: `cargo install cargo-deny --locked`, then `cargo deny check`. CI runs it in `.github/workflows/deny.yml`. `deny.toml` records four advisory ignores (all "unmaintained" notices on transitive dependencies, each with a reason).

Core tests use `wiremock` mock HTTP servers and `tempfile` directories, so they run offline.

### End-to-end tests against a real Ollama

`crates/hf2ollama-core/tests/e2e_real_ollama.rs` holds `#[ignore]`d tests that import tiny public models (`ggml-org/tiny-llamas`, `hf-internal-testing/tiny-random-LlamaForCausalLM`) through the real pipeline. They need network access to huggingface.co (no token) and an Ollama URL in `HF2OLLAMA_E2E_OLLAMA_URL`. Use a throwaway server, not your everyday one; the tests refuse port 11434 unless `HF2OLLAMA_E2E_ALLOW_DEFAULT=1`:

```sh
OLLAMA_HOST=127.0.0.1:11555 OLLAMA_MODELS="$(mktemp -d)" ollama serve &
HF2OLLAMA_E2E_OLLAMA_URL=http://127.0.0.1:11555 \
  cargo test -p hf2ollama-core --test e2e_real_ollama -- --ignored --test-threads=1
```

They create and delete models named `hf2o-e2e-*` only.

## Running the app

```sh
cargo run -p hf2ollama-gui
cargo run -p hf2ollama-gui --release
```

The GUI is under construction, so what you see depends on how far it has got. To try portable mode, create an empty file named `portable` next to the built executable (for example `target/release/portable`).

### Taking screenshots

Screenshot checks of the main views (Browse, Installed, Settings, in light and dark, at the default window size and a narrow size) are done manually for v1. Run the app, set the theme in Settings, and use your OS screenshot tool on the app window. Compare each against the matching view in `docs/design/ui-mockup.html`. Do not commit screenshots unless a doc needs one.

## How the HTML mockup is used

`docs/design/ui-mockup.html` is a standalone page you can open in any browser. It is the visual source of truth for the GUI: layout, spacing, copy and the design tokens for light and dark themes (the spec lists the token values). The egui `theme` module re-creates those tokens, and the fonts are the same ones the mockup uses. When the spec and the mockup disagree on how something looks, the mockup wins; on behavior, the spec wins. The mockup is marked as documentation in `.gitattributes` so it does not count as project source.

## Test-driven development in practice

Every change is two commits. Here is a real pair from this repository's log:

```
test(core): hardware detection      <- failing tests only, committed first
feat(core): hardware detection      <- implementation that makes them pass
```

and a bug fix, which starts with a test that reproduces the bug:

```
test(core): cancel versus create outcomes
fix(core): completed create wins over cancel; do not abandon in-flight create
```

The workflow:

1. Write the test. Run `cargo test` and watch it fail for the expected reason.
2. Commit it as `test(scope): what it covers`. (Formatting and clippy still need to pass, so a test that references not-yet-existing items is usually committed with a minimal stub that makes it compile and fail.)
3. Write the smallest implementation that passes. Run the three checks.
4. Commit as `feat(scope): ...` or `fix(scope): ...`.

Use `git log --oneline` to see the pattern.

## How the verified-API fixtures were recorded

The files in `crates/hf2ollama-core/tests/fixtures/` (`hf_search.json`, `hf_model_gguf.json`, `hf_model_safetensors.json`, `hf_model_gated.json`, `ollama_version.json`, `ollama_tags.json`, `ollama_show.json`) are real responses captured on 2026-10-22 from the public Hugging Face API without a token and from a local Ollama 0.40.8 on macOS. The test repos were a tiny GGUF repo, a tiny random Llama safetensors repo and a gated repo. The real import checks used a temporary Ollama model that was deleted afterwards. The method, the findings and the differences from the original spec are written up in `docs/superpowers/notes/api-verification.md`.

To re-record, request the same endpoints (for Hugging Face, `GET /api/models/<repo>?blobs=true`; for Ollama, `/api/version`, `/api/tags` and `/api/show`), save the JSON body, and update the tests that parse it. Check the response for anything private before committing; never commit a token.

## Updating dependencies

Dependabot proposes cargo and GitHub Actions updates weekly. Dependencies must keep the tree free of OpenSSL and native-tls and within the license allow-list in `deny.toml`.

## Screenshot and test hooks

These environment variables only change how the GUI starts, and are always available:

- `HF2O_THEME=light|dark` overrides the saved theme.
- `HF2O_SIZE=WxH` sets the initial window size (not below 900x640).
- `HF2O_VIEW=browse|installed|settings` starts on that view.

The token hooks exist only when the GUI is built with the `dev-hooks` cargo feature (off by default, so release binaries cannot contain them):

```
HF2O_MEMORY_TOKEN_STORE=1 HF2O_MEMORY_TOKEN=hf_example \
  cargo run -p hf2ollama-gui --release --features dev-hooks
```

- `HF2O_MEMORY_TOKEN_STORE=1` (exactly `1`) keeps the Hugging Face token in memory instead of the system keychain. The app prints a warning at startup and Settings shows "dev: token not persisted".
- `HF2O_MEMORY_TOKEN` is the token the in-memory store starts with.

## Releases

Merging code to `main` publishes a release automatically (`.github/workflows/release.yml`). Docs-only merges (Markdown, `docs/`, licences) do not. The workflow runs the full CI first, then builds for macOS (Apple Silicon and Intel), Windows and Linux, and creates a GitHub release with the archives and a `SHA256SUMS.txt`.

Versions are `YY.M.N`: the year without the century, the month without a leading zero, and the number of the release in that month. The first release of October 2026 is `26.10.1`, the next `26.10.2`, and the first of November `26.11.1`. The number is found by counting the existing `vYY.M.*` tags. The version is written into the crate manifests only inside the build, so the checked-in `Cargo.toml` files keep their own numbers. To release without a code change, run the workflow by hand from the Actions tab.

The builds are not signed or notarized, so macOS and Windows warn on first launch; the release notes say how to get past that.
