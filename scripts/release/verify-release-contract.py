#!/usr/bin/env python3
"""Fail closed when a release ref does not match the package contract."""

from __future__ import annotations

import argparse
import re
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def fail(message: str) -> None:
    raise SystemExit(f"release contract violation: {message}")


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
    if package.get("license") != "AGPL-3.0-only":
        fail("Cargo package must declare AGPL-3.0-only")
    if package.get("repository") != "https://github.com/IngvarConsulting/v8-runner-rust":
        fail("Cargo package must name the maintained fork repository")

    for required in ("Cargo.lock", "LICENSE", "FORK_NOTICE.md", ".github/workflows/release.yml"):
        if not (ROOT / required).is_file():
            fail(f"required corresponding-source file is missing: {required}")

    status = subprocess.run(
        ["git", "status", "--porcelain"], cwd=ROOT, check=True,
        text=True, stdout=subprocess.PIPE,
    ).stdout
    if status:
        fail("release checkout must be clean")

    print(f"release contract verified for {args.tag}")


if __name__ == "__main__":
    main()
