#!/usr/bin/env python3
"""Check the release source, package metadata, and GitHub tag signature."""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
from pathlib import Path
from typing import NoReturn
from urllib.parse import quote

ROOT = Path(__file__).resolve().parents[2]
RELEASE_REPOSITORY = "IngvarConsulting/v8-runner-rust"
MIN_CONSOLIDATED_MANIFEST_VERSION = (0, 7, 0)
PRERELEASE_IDENTIFIER = (
    r"(?:0|[1-9][0-9]*|[0-9A-Za-z-]*[A-Za-z-][0-9A-Za-z-]*)"
)
SEMVER_PATTERN = re.compile(
    r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)"
    rf"(?:-{PRERELEASE_IDENTIFIER}(?:\.{PRERELEASE_IDENTIFIER})*)?"
    r"(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?"
)


def fail(message: str) -> NoReturn:
    raise SystemExit(f"release contract violation: {message}")


def git_revision(name: str) -> str:
    return subprocess.run(
        ["git", "rev-parse", name],
        cwd=ROOT,
        check=True,
        text=True,
        stdout=subprocess.PIPE,
    ).stdout.strip()


def github_object(endpoint: str) -> dict:
    try:
        result = subprocess.run(
            ["gh", "api", "--hostname", "github.com", f"repos/{RELEASE_REPOSITORY}/{endpoint}"],
            cwd=ROOT, check=True, stdout=subprocess.PIPE, encoding="utf-8", timeout=30,
        )
        data = json.loads(result.stdout)
    except (OSError, subprocess.SubprocessError, ValueError):
        fail(f"could not read GitHub API response for {endpoint}")
    if not isinstance(data, dict):
        fail(f"expected a GitHub API object for {endpoint}")
    return data


def require_verified_release_tag(tag: str, commit: str) -> None:
    ref = f"refs/tags/{tag}"
    reference = github_object(f"git/ref/tags/{quote(tag, safe='')}")
    target = reference.get("object")
    if reference.get("ref") != ref or not isinstance(target, dict) or target.get("type") != "tag":
        fail(f"{tag} must be an annotated tag on GitHub")

    try:
        tag_sha = git_revision(f"{ref}^{{tag}}")
    except subprocess.CalledProcessError:
        fail(f"{tag} must be an annotated tag in the checkout")
    if target.get("sha") != tag_sha:
        fail(f"{tag} points to a different tag object on GitHub than in the checkout")

    annotated = github_object(f"git/tags/{tag_sha}")
    if annotated.get("sha") != tag_sha or annotated.get("tag") != tag:
        fail(f"GitHub returned a different tag object for {tag}")
    destination = annotated.get("object")
    if (
        not isinstance(destination, dict)
        or destination.get("type") != "commit"
        or destination.get("sha") != commit
    ):
        fail(f"annotated tag {tag} must point directly to the approved source commit {commit}")

    verification = annotated.get("verification")
    if not isinstance(verification, dict) or verification.get("verified") is not True:
        reason = verification.get("reason") if isinstance(verification, dict) else None
        fail(f"{tag} requires GitHub verification.verified=true (reason: {reason!r})")


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
    if package.get("repository") != f"https://github.com/{RELEASE_REPOSITORY}":
        fail("Cargo package must name the maintained fork repository")
    if os.environ.get("GITHUB_REPOSITORY", "").lower() != RELEASE_REPOSITORY.lower():
        fail(f"release workflow must run in {RELEASE_REPOSITORY}")

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

    require_verified_release_tag(args.tag, head)
    print(f"release contract verified for {args.tag}")


if __name__ == "__main__":
    main()
