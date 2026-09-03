#!/usr/bin/env python3
"""Fail closed when a release ref does not match the package contract."""

from __future__ import annotations

import argparse
import os
import re
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MIN_CONSOLIDATED_MANIFEST_VERSION = (0, 7, 0)
PRERELEASE_IDENTIFIER = (
    r"(?:0|[1-9][0-9]*|[0-9A-Za-z-]*[A-Za-z-][0-9A-Za-z-]*)"
)
SEMVER_PATTERN = re.compile(
    r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)"
    rf"(?:-{PRERELEASE_IDENTIFIER}(?:\.{PRERELEASE_IDENTIFIER})*)?"
    r"(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?"
)


def fail(message: str) -> None:
    raise SystemExit(f"release contract violation: {message}")


def git_revision(name: str) -> str:
    return subprocess.run(
        ["git", "rev-parse", name],
        cwd=ROOT,
        check=True,
        text=True,
        stdout=subprocess.PIPE,
    ).stdout.strip()


def require_consolidated_manifest_version(version: str) -> None:
    match = SEMVER_PATTERN.fullmatch(version)
    if match is None:
        fail(f"Cargo package version {version!r} must be a valid semantic version")
    package_version = tuple(int(component) for component in match.groups())
    if package_version < MIN_CONSOLIDATED_MANIFEST_VERSION:
        fail("consolidated release assets require v0.7.0 or newer")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("tag")
    args = parser.parse_args()

    cargo = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    package_section = cargo.split("[package]", 1)[1].split("\n[", 1)[0]

    def package_value(name: str) -> str | None:
        match = re.search(rf'^\s*{re.escape(name)}\s*=\s*"([^"]+)"\s*$', package_section, re.M)
        return match.group(1) if match else None

    package = {
        "version": package_value("version"),
        "license": package_value("license"),
        "repository": package_value("repository"),
    }
    expected_tag = f"v{package['version']}"
    if args.tag != expected_tag:
        fail(f"tag {args.tag!r} must equal Cargo package tag {expected_tag!r}")
    require_consolidated_manifest_version(package["version"] or "")
    if package.get("license") != "AGPL-3.0-only":
        fail("Cargo package must declare AGPL-3.0-only")
    if package.get("repository") != "https://github.com/IngvarConsulting/v8-runner-rust":
        fail("Cargo package must name the maintained fork repository")

    for required in (
        "Cargo.lock",
        "LICENSE",
        "FORK_NOTICE.md",
        ".github/workflows/release.yml",
        "scripts/release/release_assets.py",
    ):
        if not (ROOT / required).is_file():
            fail(f"required corresponding-source file is missing: {required}")

    status = subprocess.run(
        ["git", "status", "--porcelain"], cwd=ROOT, check=True,
        text=True, stdout=subprocess.PIPE,
    ).stdout
    if status:
        fail("release checkout must be clean")

    head = git_revision("HEAD")
    tag_commit = git_revision(f"refs/tags/{args.tag}^{{commit}}")
    master = git_revision("refs/remotes/origin/master")
    workflow_commit = os.environ.get("GITHUB_SHA")
    if not workflow_commit:
        fail("GITHUB_SHA is required to bind the approved workflow commit")
    if len({head, tag_commit, master, workflow_commit}) != 1:
        fail(
            "release source identity must match HEAD, tag commit, protected origin/master, "
            f"and GITHUB_SHA: HEAD={head}, tag={tag_commit}, master={master}, workflow={workflow_commit}"
        )

    print(f"release contract verified for {args.tag}")


if __name__ == "__main__":
    main()
