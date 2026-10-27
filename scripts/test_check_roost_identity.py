#!/usr/bin/env python3
"""Characterization tests for the legacy identity scanner."""
import importlib.util
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location("identity_scan", Path(__file__).with_name("check_roost_identity.py"))
scan = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(scan)


class IdentityScannerTests(unittest.TestCase):
    def test_matches_product_and_legacy_environment_prefixes(self):
        self.assertTrue(scan.matches_legacy("hf2ollama"))
        self.assertTrue(scan.matches_legacy("HF2O_THEME"))
        self.assertTrue(scan.matches_legacy("hf2o_memory_token"))
        self.assertFalse(scan.matches_legacy("ROOST_THEME"))

    def test_only_explicit_allowance_accepts_legacy_line(self):
        self.assertTrue(scan.allowed_line("crates/roost-gui/src/settings/mod.rs", 'RichText::new("Import hf2ollama token")'))
        self.assertFalse(scan.allowed_line("crates/roost-gui/src/settings/mod.rs", 'RichText::new("hf2ollama elsewhere")'))


if __name__ == "__main__":
    unittest.main()
