#!/usr/bin/env python3
"""Reject legacy product/environment references outside precise compatibility records."""
from pathlib import Path
import re
import subprocess
import sys

LEGACY = re.compile(r"hf2ollama|hf2o_", re.IGNORECASE)
# Immutable dated records and the current migration design that discusses legacy
# names are allowed in full. All executable compatibility paths use line rules.
ALLOW_FILE = {
    "CHANGELOG.md",
    "docs/design/egui-spike-reference.rs",
    "docs/superpowers/notes/acceptance.md",
    "docs/superpowers/notes/api-verification.md",
    "docs/superpowers/notes/build-ledger.md",
    "docs/superpowers/notes/handoff.md",
    "docs/superpowers/plans/2026-10-22-hf2ollama.md",
    "docs/superpowers/specs/2026-10-22-hf2ollama-design.md",
    "docs/superpowers/plans/2026-10-27-roost-foundation.md",
    "docs/superpowers/specs/2026-10-27-roost-workbench-design.md",
}
ALLOW_LINE = {
    "crates/roost-core/src/migration.rs": (r"hf2ollama metadata",),
    "crates/roost-core/src/paths.rs": (r"former hf2ollama", r'Self::project\("hf2ollama"\)', r"preceding hf2ollama", r'join\("hf2ollama\.toml"\)'),
    "crates/roost-core/src/secrets.rs": (r'LEGACY_SERVICE: &str = "hf2ollama"', r"legacy hf2ollama keychain"),
    "crates/roost-core/src/staging.rs": (r'"\.hf2ollama-job"',),
    "crates/roost-core/src/config.rs": (r'join\("hf2ollama\.toml"\)',),
    "crates/roost-core/tests/migration.rs": (r"hf2ollama",),
    "crates/roost-core/tests/e2e_real_ollama.rs": (r"hf2o-e2e-",),
    "crates/roost-gui/src/startup.rs": (r'join\("config/hf2ollama\.toml"\)', r'join\("hf2ollama\.toml"\)'),
    "crates/roost-gui/src/runtime.rs": (r"legacy hf2ollama keychain",),
    "crates/roost-gui/src/settings/mod.rs": (r'Import hf2ollama token',),
    "crates/roost-gui/src/main.rs": (r'"HF2O_(THEME|SIZE|QUERY|SELECT|VIEW)"',),
    "README.md": (r"Coming from hf2ollama",),
    "docs/ARCHITECTURE.md": (r"2026-10-22-hf2ollama-design", r"import of hf2ollama files", r"from the hf2ollama locations"),
    "docs/DEVELOPMENT.md": (r"HF2O_\*",),
    "docs/FAQ.md": (r"I used hf2ollama before", r"copies your hf2ollama", r"Import hf2ollama token"),
    "docs/ROADMAP.md": (r"2026-10-22-hf2ollama-design", r"migration of hf2ollama settings"),
    "scripts/check_roost_identity.py": (r"hf2ollama|hf2o_",),
    "scripts/test_check_roost_identity.py": (r"hf2ollama|HF2O_|hf2o_",),
}

def matches_legacy(text):
    return bool(LEGACY.search(text))

def allowed_line(name, line):
    if name in ALLOW_FILE:
        return True
    return any(re.search(pattern, line, re.IGNORECASE) for pattern in ALLOW_LINE.get(name, ()))

def main():
    files = subprocess.check_output(["git", "ls-files"], text=True).splitlines()
    violations = []
    for name in files:
        try:
            lines = Path(name).read_text(encoding="utf-8").splitlines()
        except UnicodeDecodeError:
            continue
        for number, line in enumerate(lines, 1):
            if matches_legacy(line) and not allowed_line(name, line):
                violations.append(f"{name}:{number}: {line.strip()}")
    if violations:
        print("unapproved legacy identity references:", *violations, sep="\n  ", file=sys.stderr)
        return 1
    print("Roost identity scan passed")
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
