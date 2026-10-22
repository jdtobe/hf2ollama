## What and why

<!-- What does this change, and why? Link the issue if there is one. -->

## Checklist

- [ ] A failing `test(scope): ...` commit comes before the `feat(scope): ...` or `fix(scope): ...` commit (strict TDD).
- [ ] Commit messages use conventional prefixes (`feat`, `fix`, `test`, `docs`, `chore`, `ci`, `refactor`) and scope `core` or `gui` where it applies.
- [ ] `cargo fmt --check` passes.
- [ ] `cargo clippy --all-targets -- -D warnings` passes.
- [ ] `cargo test` passes (no network needed).
- [ ] No `unwrap` or `expect` on I/O, network or parse results in non-test code.
- [ ] No OpenSSL or native-tls in the dependency tree (rustls only); `cargo deny check` passes if dependencies changed.
- [ ] Core does not import egui, and the GUI makes no HTTP calls itself.
- [ ] Docs and `CHANGELOG.md` (`[Unreleased]`) updated if behavior changed.
- [ ] For UI changes: checked against `docs/design/ui-mockup.html` in light and dark themes.
