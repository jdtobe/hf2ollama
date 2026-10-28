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
  roost-core/             library, no UI dependencies
    src/                      modules described in docs/ARCHITECTURE.md
    src/supported_architectures.json   bundled data
    tests/fixtures/           recorded Hugging Face and Ollama responses
  roost-gui/              egui/eframe binary
    assets/                   bundled fonts and their OFL license texts
docs/
  ARCHITECTURE.md FAQ.md DEVELOPMENT.md ROADMAP.md
  design/ui-mockup.html       approved visual reference
  design/egui-spike-reference.rs   egui spike used as a style reference
  superpowers/specs/          the design spec (source of truth)
  superpowers/plans/          the implementation plan
  superpowers/notes/          api-verification.md, acceptance.md
packaging/
  macos/                      .app/.dmg scripts, Info.plist template and icon generator
.github/                      CI, cargo-deny workflow, templates, dependabot
deny.toml                     cargo-deny policy
```

## Running the checks

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

CI also runs clippy and the tests with `--features roost-gui/dev-hooks`.

All three must pass before each commit. Run a single crate or test while iterating:

```sh
cargo test -p roost-core
cargo test -p roost-core fit
```

Optional, if you have it installed: `cargo install cargo-deny --locked`, then `cargo deny check`. CI runs it in `.github/workflows/deny.yml`. `deny.toml` records four advisory ignores (all "unmaintained" notices on transitive dependencies, each with a reason).

Core tests use `wiremock` mock HTTP servers and `tempfile` directories, so they run offline.

### End-to-end tests against a real Ollama

`crates/roost-core/tests/e2e_real_ollama.rs` holds `#[ignore]`d tests that import tiny public models (`ggml-org/tiny-llamas`, `hf-internal-testing/tiny-random-LlamaForCausalLM`) through the real pipeline. They need network access to huggingface.co (no token) and an Ollama URL in `ROOST_E2E_OLLAMA_URL`. Use a throwaway server, not your everyday one; the tests refuse port 11434 unless `ROOST_E2E_ALLOW_DEFAULT=1`:

```sh
OLLAMA_HOST=127.0.0.1:11555 OLLAMA_MODELS="$(mktemp -d)" ollama serve &
ROOST_E2E_OLLAMA_URL=http://127.0.0.1:11555 \
  cargo test -p roost-core --test e2e_real_ollama -- --ignored --test-threads=1
```

They create and delete models named `hf2o-e2e-*` only.

## Running the app

```sh
cargo run -p roost-gui
cargo run -p roost-gui --release
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

The files in `crates/roost-core/tests/fixtures/` (`hf_search.json`, `hf_model_gguf.json`, `hf_model_safetensors.json`, `hf_model_gated.json`, `ollama_version.json`, `ollama_tags.json`, `ollama_show.json`) are real responses captured on 2026-10-22 from the public Hugging Face API without a token and from a local Ollama 0.40.8 on macOS. The test repos were a tiny GGUF repo, a tiny random Llama safetensors repo and a gated repo. The real import checks used a temporary Ollama model that was deleted afterwards. The method, the findings and the differences from the original spec are written up in `docs/superpowers/notes/api-verification.md`.

To re-record, request the same endpoints (for Hugging Face, `GET /api/models/<repo>?blobs=true`; for Ollama, `/api/version`, `/api/tags` and `/api/show`), save the JSON body, and update the tests that parse it. Check the response for anything private before committing; never commit a token.

## Updating dependencies

Dependabot proposes cargo and GitHub Actions updates weekly. Dependencies must keep the tree free of OpenSSL and native-tls and within the license allow-list in `deny.toml`.

## Screenshot and test hooks

These environment variables only change how the GUI starts, and are always available:

- `ROOST_THEME` overrides the saved theme: `light` or `dark` change the mode only; `meadow-light`, `meadow-dark`, `ember-light`, `ember-dark`, `plum-light`, `plum-dark`, `tide-light` or `tide-dark` set family and mode.
- `ROOST_SIZE=WxH` sets the initial window size (not below 900x640).
- `ROOST_VIEW=browse|installed|settings` starts on that view.

The token hooks exist only when the GUI is built with the `dev-hooks` cargo feature (off by default, so release binaries cannot contain them):

```
ROOST_MEMORY_TOKEN_STORE=1 ROOST_MEMORY_TOKEN=hf_example \
  cargo run -p roost-gui --release --features dev-hooks
```

- `ROOST_MEMORY_TOKEN_STORE=1` (exactly `1`) keeps the Hugging Face token in memory instead of the system keychain. The app prints a warning at startup and Settings shows "dev: token not persisted".
- `ROOST_MEMORY_TOKEN` is the token the in-memory store starts with.

## Releases

Releases are manual for now (`.github/workflows/release.yml`, run from the Actions tab with **Run workflow**). The workflow runs the full CI, then builds for macOS (Apple Silicon and Intel), Windows and Linux. With **publish** ticked, and only when run on `main`, it creates a GitHub release with the archives and a `SHA256SUMS.txt`. With it unticked it is a dry run: the archives are kept as run artifacts and nothing is published.

GitHub only lists a manually run workflow once its file is on the default branch, so the first run is possible after the workflow has been merged to `main`.

Versions are `YY.M.N`: the year without the century, the month without a leading zero, and the number of the release in that month, counting from 0. The first release of October 2026 is `26.10.0`, the next `26.10.1`, and the first of November `26.11.0`. The number is found by counting the existing `vYY.M.*` tags. The version is written into the crate manifests only inside the build, so the checked-in `Cargo.toml` files keep their own numbers.

The comment at the top of `release.yml` says how to switch to automatic releases on every code merge to `main`.

The builds are not signed or notarized, so macOS and Windows warn on first launch; the release notes say how to get past that.

### macOS app and DMG

macOS is released as one universal `.dmg` containing `Roost.app` for Apple Silicon and Intel. `packaging/macos/package.sh` lipos the two release binaries, builds the bundle (`Info.plist`, icon, notices), applies an ad-hoc signature, creates the DMG with an `Applications` link and runs `packaging/macos/verify.sh`. The verifier checks the checksum, layout, plist values, both architectures, the ICNS icon and the signature.

To build it locally (the x86_64 target may need `rustup target add x86_64-apple-darwin`):

```sh
cargo build --release -p roost-gui --target aarch64-apple-darwin
cargo build --release -p roost-gui --target x86_64-apple-darwin
packaging/macos/package.sh 26.10.0 \
  target/aarch64-apple-darwin/release/roost \
  target/x86_64-apple-darwin/release/roost dist
```

The icon source is `crates/roost-gui/assets/icon-1024.png`; regenerate it with `python3 packaging/macos/make_icon.py`. The ad-hoc signature makes the bundle internally valid but is not Developer ID signing or notarization, so Gatekeeper still warns.
