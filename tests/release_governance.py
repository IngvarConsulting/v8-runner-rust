#!/usr/bin/env python3
"""Guard the fork-owned release and provenance contract.

Root cause: the inherited workflow could publish untested archives without the
license or fork notice and resolved third-party actions through mutable tags.
Single owner: ``.github/workflows/release.yml`` plus the package metadata named
below. This test is the reintroduction guard for equivalent release paths.
"""

from __future__ import annotations

import importlib.util
import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def load_release_verifier():
    path = ROOT / "scripts/release/verify-release-contract.py"
    spec = importlib.util.spec_from_file_location("verify_release_contract", path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class ReleaseGovernanceTest(unittest.TestCase):
    def test_release_is_verified_and_self_describing(self) -> None:
        workflow = (ROOT / ".github/workflows/release.yml").read_text(encoding="utf-8")
        self.assertIn("preflight:", workflow)
        self.assertIn("needs: preflight", workflow)
        self.assertIn("scripts/release/verify-release-contract.py", workflow)
        self.assertIn('cp LICENSE "${package_dir}/"', workflow)
        self.assertIn('cp FORK_NOTICE.md "${package_dir}/"', workflow)

    def test_release_has_one_protected_entrypoint_and_freezes_only_after_audit(self) -> None:
        workflow = (ROOT / ".github/workflows/release.yml").read_text(encoding="utf-8")
        self.assertNotIn("push:\n    tags:", workflow)
        self.assertIn("workflow_dispatch:", workflow)
        self.assertIn("group: release-${{ inputs.tag }}", workflow)
        self.assertIn("environment: release", workflow)
        self.assertIn("github.ref == 'refs/heads/master'", workflow)
        self.assertIn("name: Audit draft release", workflow)
        self.assertIn("needs: [publish, audit-native]", workflow)
        self.assertIn("gh release edit", workflow)
        self.assertIn("--draft=false", workflow)
        self.assertIn('existing_state="$(gh release view', workflow)
        self.assertIn('if [[ "${existing_state}" == "false" ]]', workflow)
        self.assertIn('if [[ "${existing_state}" == "true" ]]', workflow)
        self.assertIn('gh release delete "${RELEASE_TAG}"', workflow)

    def test_release_verification_waits_for_github_attestation_with_a_deadline(self) -> None:
        workflow = (ROOT / ".github/workflows/release.yml").read_text(encoding="utf-8")
        freeze = workflow.split("  freeze:\n", 1)[1]
        self.assertIn("RELEASE_VERIFY_TIMEOUT_SECONDS", freeze)
        self.assertIn('until gh release verify "${RELEASE_TAG}"', freeze)
        self.assertIn("SECONDS >= verify_deadline", freeze)
        self.assertIn("sleep 5", freeze)

    def test_release_publishes_attested_direct_unica_assets(self) -> None:
        workflow = (ROOT / ".github/workflows/release.yml").read_text(encoding="utf-8")
        for asset in (
            "v8-runner-darwin-arm64",
            "v8-runner-linux-x64",
            "v8-runner-win-x64.exe",
        ):
            self.assertIn(asset, workflow)
        self.assertIn("actions/attest-build-provenance@e8998f949152b193b063cb0ec769d69d929409be", workflow)
        self.assertIn("id-token: write", workflow)
        self.assertIn("attestations: write", workflow)
        self.assertIn("v8-runner-assets.json", workflow)
        self.assertIn("test -f dist/v8-runner-assets.json", workflow)
        self.assertIn("license-v8-runner-AGPL-3.0-only.txt", workflow)
        self.assertIn("notice-v8-runner-fork.txt", workflow)
        self.assertIn("gh attestation verify", workflow)
        self.assertIn("--deny-self-hosted-runners", workflow)
        self.assertIn('chmod +x "dist/${{ matrix.asset }}"', workflow)

    def test_release_publishes_one_manifest_instead_of_per_asset_sidecars(self) -> None:
        workflow = (ROOT / ".github/workflows/release.yml").read_text(encoding="utf-8")
        self.assertNotIn("write-checksum", workflow)
        self.assertNotRegex(workflow, r"dist/[^\n]*(?:\.sha256|\.provenance\.json)")
        self.assertIn("v8-runner-assets.json", workflow)

    def test_portable_archive_documents_have_canonical_line_endings(self) -> None:
        attributes = (ROOT / ".gitattributes").read_text(encoding="utf-8")
        for name in ("README.md", "LICENSE", "FORK_NOTICE.md"):
            self.assertIn(f"{name} text eol=lf", attributes)

    def test_all_payload_assets_and_manifest_have_build_attestations(self) -> None:
        workflow = (ROOT / ".github/workflows/release.yml").read_text(encoding="utf-8")
        self.assertIn("Attest portable archive", workflow)
        self.assertIn("Attest direct Unica asset", workflow)
        self.assertIn("Attest consolidated release manifest", workflow)
        self.assertIn("for asset in $(python3 scripts/release/release_assets.py attested-assets)", workflow)

    def test_draft_auditors_can_read_unpublished_release(self) -> None:
        workflow = (ROOT / ".github/workflows/release.yml").read_text(encoding="utf-8")
        native = workflow.split("  audit-native:\n", 1)[1].split("  audit-draft:\n", 1)[0]
        draft = workflow.split("  audit-draft:\n", 1)[1].split("  freeze:\n", 1)[0]
        self.assertIn("contents: write", native)
        self.assertIn("contents: write", draft)

    def test_release_toolchain_and_source_identity_are_pinned(self) -> None:
        workflow = (ROOT / ".github/workflows/release.yml").read_text(encoding="utf-8")
        verifier = (ROOT / "scripts/release/verify-release-contract.py").read_text(
            encoding="utf-8"
        )
        self.assertIn('toolchain: "1.95.0"', workflow)
        self.assertIn("MACOSX_DEPLOYMENT_TARGET", workflow)
        self.assertIn("refs/remotes/origin/master", verifier)
        self.assertIn("refs/tags/{args.tag}^{{commit}}", verifier)
        self.assertIn("GITHUB_SHA", verifier)
        self.assertIn("MIN_CONSOLIDATED_MANIFEST_VERSION", verifier)
        self.assertIn("consolidated release assets require", verifier)

    def test_consolidated_contract_accepts_v07_prereleases_only(self) -> None:
        verifier = load_release_verifier()

        verifier.require_consolidated_manifest_version("0.7.0-pre.1")
        verifier.require_consolidated_manifest_version("0.7.0-ic.1")
        verifier.require_consolidated_manifest_version("0.8.0+build.1")
        with self.assertRaisesRegex(SystemExit, "v0.7.0 or newer"):
            verifier.require_consolidated_manifest_version("0.6.99")
        with self.assertRaisesRegex(SystemExit, "semantic version"):
            verifier.require_consolidated_manifest_version("0.7.0-01")

    def test_documented_attestation_is_bound_to_verified_manifest_commit(self) -> None:
        readme = (ROOT / "README.md").read_text(encoding="utf-8")
        self.assertIn("gh release verify-asset v0.7.0 ./v8-runner-assets.json", readme)
        self.assertIn('source_commit="$(python3', readme)
        self.assertIn("for asset in v8-runner-assets.json v8-runner-linux-x64", readme)
        self.assertIn('--source-digest "$source_commit"', readme)

    def test_pr_ci_runs_release_asset_contract_tests(self) -> None:
        ci = (ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8")
        self.assertIn("python3 tests/release_governance.py", ci)
        self.assertIn("python3 tests/release_assets.py", ci)

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
