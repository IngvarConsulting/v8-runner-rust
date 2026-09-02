#!/usr/bin/env python3
"""Guard the fork-owned release and provenance contract.

Root cause: the inherited workflow could publish untested archives without the
license or fork notice and resolved third-party actions through mutable tags.
Single owner: ``.github/workflows/release.yml`` plus the package metadata named
below. This test is the reintroduction guard for equivalent release paths.
"""

from __future__ import annotations

import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


class ReleaseGovernanceTest(unittest.TestCase):
    def test_release_is_verified_and_self_describing(self) -> None:
        workflow = (ROOT / ".github/workflows/release.yml").read_text(encoding="utf-8")
        self.assertIn("preflight:", workflow)
        self.assertIn("needs: preflight", workflow)
        self.assertIn("scripts/release/verify-release-contract.py", workflow)
        self.assertIn('cp LICENSE "${package_dir}/"', workflow)
        self.assertIn('cp FORK_NOTICE.md "${package_dir}/"', workflow)

    def test_all_actions_are_pinned_to_full_commit_sha(self) -> None:
        for path in sorted((ROOT / ".github/workflows").glob("*.yml")):
            workflow = path.read_text(encoding="utf-8")
            floating = re.findall(r"^\s*uses:\s*[^\s@]+@(?![0-9a-f]{40}(?:\s|$))[^\s]+", workflow, re.M)
            self.assertEqual([], floating, f"floating action refs in {path}: {floating}")

    def test_package_metadata_names_fork_and_license(self) -> None:
        cargo = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
        self.assertIn('license = "AGPL-3.0-only"', cargo)
        self.assertIn('repository = "https://github.com/IngvarConsulting/v8-runner-rust"', cargo)
        self.assertTrue((ROOT / "FORK_NOTICE.md").is_file())

    def test_generated_schema_contract_no_longer_points_to_old_owner(self) -> None:
        tracked = [
            ROOT / "src/config/schema.rs",
            ROOT / "src/use_cases/config_init.rs",
            ROOT / "src/use_cases/tools_download.rs",
            ROOT / "docs/CONFIGURATION.md",
            ROOT / "docs/schemas/v8project.schema.json",
            ROOT / "docs/schemas/v8project.local.schema.json",
            ROOT / "tests/cli_bootstrap.rs",
            ROOT / "tests/cli_config_init.rs",
        ]
        for path in tracked:
            self.assertNotIn("alkoleft/v8-runner-rust", path.read_text(encoding="utf-8"), str(path))


if __name__ == "__main__":
    unittest.main()
