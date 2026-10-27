#!/usr/bin/env python3
"""Reject unapproved legacy product references in current project identity."""
from pathlib import Path
import subprocess
import sys

NEEDLE = "hf2ollama"
# Each exception is compatibility code, its direct tests, or an immutable 2026-10-22 record.
ALLOW_EXACT = {
    "crates/roost-core/src/migration.rs", "crates/roost-core/src/paths.rs",
    "crates/roost-core/src/secrets.rs", "crates/roost-core/src/staging.rs",
    "crates/roost-core/src/config.rs", "crates/roost-core/tests/migration.rs",
    "crates/roost-core/tests/e2e_real_ollama.rs", "crates/roost-gui/src/startup.rs",
    "crates/roost-gui/src/runtime.rs", "crates/roost-gui/src/settings/mod.rs", "scripts/check_roost_identity.py",
    "docs/design/egui-spike-reference.rs", "docs/superpowers/notes/acceptance.md",
    "docs/superpowers/notes/api-verification.md", "docs/superpowers/notes/build-ledger.md",
    "docs/superpowers/notes/handoff.md", "docs/superpowers/plans/2026-10-22-hf2ollama.md",
    "docs/superpowers/specs/2026-10-22-hf2ollama-design.md", "CHANGELOG.md",
    "docs/superpowers/plans/2026-10-27-roost-foundation.md",
    "docs/superpowers/specs/2026-10-27-roost-workbench-design.md",
}
ALLOW_PREFIXES = (".superpowers/",)
files = subprocess.check_output(["git", "ls-files"], text=True).splitlines()
violations = []
for name in files:
    if name in ALLOW_EXACT or name.startswith(ALLOW_PREFIXES):
        continue
    try:
        text = Path(name).read_text()
    except UnicodeDecodeError:
        continue
    if NEEDLE.lower() in text.lower():
        violations.append(name)
if violations:
    print("unapproved legacy identity references:", *violations, sep="\n  ", file=sys.stderr)
    sys.exit(1)
print("Roost identity scan passed")
