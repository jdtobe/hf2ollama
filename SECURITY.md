# Security policy

## Status

Roost is pre-release software and has no supported releases yet. Security fixes are made on the default branch.

## Reporting a vulnerability

Please do not report security problems in a public issue or discussion.

Use GitHub private vulnerability reporting: open the repository's Security tab and choose "Report a vulnerability". <!-- TODO(owner): enable private vulnerability reporting in the repository settings (Settings, Code security) so this link works. -->

If that option is not available, open a minimal public issue that says only that you have a security report and asks for a private contact. Do not include details in that issue.

Helpful details in a report: what you found, steps to reproduce, the affected version or commit, your platform, and the Ollama version if relevant.

## What to expect

This is a small project. Reports are handled on a best-effort basis, with no guaranteed response time. Valid reports will be fixed and noted in [CHANGELOG.md](CHANGELOG.md), crediting you if you want.

## Scope and areas of interest

Things this app handles that are worth reporting:

- The Hugging Face access token, which is stored in the system keychain and must not appear in logs, debug output or settings files.
- Handling of downloaded files: checksum verification, path handling in file names from repositories, and cleanup of staging folders.
- Requests to the Ollama address and Hugging Face endpoint configured by the user.
- The dependency tree: TLS is rustls only, and OpenSSL and native-tls are banned in `deny.toml`. `cargo-deny` also checks RustSec advisories in CI.

Not in scope: vulnerabilities in Ollama or Hugging Face themselves (report those to their maintainers), and the contents of models you choose to download.
