# Contributing to hf2ollama

Thanks for your interest. The project is pre-release and the GUI is still being built, so check [docs/ROADMAP.md](docs/ROADMAP.md) and open an issue before starting anything large. By taking part you agree to follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## Getting set up

You need stable Rust (with `rustfmt` and `clippy`). On Linux, install `libxkbcommon-dev`, `libwayland-dev` and `libgtk-3-dev` to build the GUI crate. See [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) for the repo tour and how to run things, and [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for how the crates fit together.

## The checks that must pass

Before every commit, run all three and fix what they report:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

CI runs the same commands on macOS, Windows and Linux. Tests must not need a network; use the mock HTTP servers and the recorded fixtures in `crates/hf2ollama-core/tests/fixtures/`.

## Test-driven development

This project uses strict TDD, and the history shows it:

1. Write a failing test and commit it as `test(scope): ...`.
2. Write the smallest implementation that makes it pass and commit it as `feat(scope): ...` (or `fix(scope): ...` for a bug).

The test commit comes before the implementation commit. For a bug fix, the failing test reproduces the bug first. See [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) for a concrete example from the log.

## Commit messages

Use conventional commit prefixes: `feat`, `fix`, `test`, `docs`, `chore`, `ci`, `refactor`. Use a scope when the change belongs to one crate: `core` or `gui`. Examples:

```
test(core): hardware detection
feat(core): hardware detection
fix(core): restart when 206 Content-Range does not match resume offset
docs: verified Hugging Face and Ollama API behavior
ci: build, fmt, clippy and test on macOS, Windows and Linux
```

Keep commits small and focused. Do not rewrite shared history.

## Code rules

- No `unwrap` or `expect` on I/O, network or parse results in non-test code. Return typed errors (`thiserror`) instead. Tests may unwrap.
- TLS is rustls only. OpenSSL must never enter the dependency tree. `deny.toml` bans `openssl`, `openssl-sys` and `native-tls`; when you add a dependency, disable default features that pull them in.
- `hf2ollama-core` never imports egui. The GUI never makes HTTP calls itself; it sends commands to core and receives events.
- The UI thread must never block on I/O.
- Keep bundled data (such as the supported-architecture list) as data files, not code.
- Do not add dependencies casually. Prefer permissive licenses; `cargo deny check` must pass.

## Pull requests

- Branch from `main` and keep the PR to one topic.
- Fill in the pull request template checklist.
- Include the tests, and update docs and `CHANGELOG.md` (under `[Unreleased]`) when behavior changes.
- For UI changes, say which view and theme you checked against `docs/design/ui-mockup.html`.

## Reporting bugs and requesting features

Use the issue templates. For security problems, do not open a public issue; follow [SECURITY.md](SECURITY.md).

## License

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this project, as defined in the Apache-2.0 license, is dual licensed under MIT OR Apache-2.0 as described in the [README](README.md#license), without any additional terms or conditions.
