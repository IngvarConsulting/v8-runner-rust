#!/usr/bin/env python3
"""Build and verify the fork-owned direct release asset contract."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import struct
import subprocess
from pathlib import Path


REPOSITORY = "https://github.com/IngvarConsulting/v8-runner-rust"
DIRECT_ASSETS = {
    "aarch64-apple-darwin": "v8-runner-darwin-arm64",
    "x86_64-pc-windows-msvc": "v8-runner-win-x64.exe",
    "x86_64-unknown-linux-musl": "v8-runner-linux-x64",
}
ARCHIVE_ASSETS = {
    "v8-runner-linux-x86_64-musl.tar.gz",
    "v8-runner-macos-aarch64.tar.gz",
    "v8-runner-macos-x86_64.tar.gz",
    "v8-runner-windows-x86_64.zip",
}
METADATA_ASSETS = {
    "license-v8-runner-AGPL-3.0-only.txt",
    "notice-v8-runner-fork.txt",
    "v8-runner-assets.json",
}


def digest(path: Path) -> str:
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(chunk)
    return value.hexdigest()


def validate_binary_bytes(data: bytes, target: str) -> None:
    if target == "x86_64-unknown-linux-musl":
        if len(data) < 64 or data[:6] != b"\x7fELF\x02\x01" or struct.unpack_from("<H", data, 18)[0] != 62:
            raise ValueError("expected little-endian ELF64 x86-64")
        phoff = struct.unpack_from("<Q", data, 32)[0]
        phentsize = struct.unpack_from("<H", data, 54)[0]
        phnum = struct.unpack_from("<H", data, 56)[0]
        if phnum and phentsize < 4:
            raise ValueError("ELF program header entry is truncated")
        if phoff + phentsize * phnum > len(data):
            raise ValueError("ELF program headers are truncated")
        if any(struct.unpack_from("<I", data, phoff + index * phentsize)[0] == 3 for index in range(phnum)):
            raise ValueError("musl release must not contain PT_INTERP")
        return

    if target == "x86_64-pc-windows-msvc":
        if len(data) < 0x40 or data[:2] != b"MZ":
            raise ValueError("expected PE executable")
        offset = struct.unpack_from("<I", data, 0x3C)[0]
        if offset + 6 > len(data) or data[offset : offset + 4] != b"PE\x00\x00" or struct.unpack_from("<H", data, offset + 4)[0] != 0x8664:
            raise ValueError("expected PE x86-64")
        return

    expected_cpu = {
        "x86_64-apple-darwin": 0x01000007,
        "aarch64-apple-darwin": 0x0100000C,
    }.get(target)
    if expected_cpu is not None:
        if data[:4] != b"\xcf\xfa\xed\xfe" or struct.unpack_from("<I", data, 4)[0] != expected_cpu:
            raise ValueError(f"expected Mach-O 64 for {target}")
        return

    raise ValueError(f"unsupported release target: {target}")


def verify_binary(path: Path, target: str, version: str) -> None:
    validate_binary_bytes(path.read_bytes(), target)
    completed = subprocess.run(
        [str(path.resolve()), "--version"],
        check=True,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
    )
    if version not in completed.stdout:
        raise ValueError(f"{path.name} --version did not report {version!r}: {completed.stdout!r}")


def checksum_file(path: Path) -> Path:
    output = path.with_name(f"{path.name}.sha256")
    output.write_text(f"{digest(path)}  {path.name}\n", encoding="utf-8")
    return output


def parse_checksum(value: str) -> tuple[str, str]:
    match = re.fullmatch(r"([0-9A-Fa-f]{64}) ([ *])([^\r\n]+)\r?\n?", value)
    if match is None:
        raise ValueError("invalid sha256sum file format")
    return match.group(1).lower(), match.group(3)


def prepare_direct(args: argparse.Namespace) -> None:
    expected = DIRECT_ASSETS.get(args.target)
    if expected != args.asset_name:
        raise ValueError(f"direct asset for {args.target} must be {expected!r}")
    source = Path(args.binary)
    output = Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, output)
    if os.name != "nt":
        output.chmod(output.stat().st_mode | 0o111)
    verify_binary(output, args.target, args.version)
    sha = digest(output)
    checksum_file(output)
    provenance = {
        "schemaVersion": 1,
        "name": output.name,
        "targetTriple": args.target,
        "size": output.stat().st_size,
        "sha256": sha,
        "sourceRepository": REPOSITORY,
        "sourceTag": args.tag,
        "sourceCommit": args.commit,
        "builderWorkflow": ".github/workflows/release.yml",
        "runnerEnvironment": "github-hosted",
    }
    output.with_name(f"{output.name}.provenance.json").write_text(
        json.dumps(provenance, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )


def expected_release_files() -> set[str]:
    direct = set(DIRECT_ASSETS.values())
    checksums = {f"{name}.sha256" for name in direct | ARCHIVE_ASSETS}
    provenance = {f"{name}.provenance.json" for name in direct}
    return direct | ARCHIVE_ASSETS | checksums | provenance | METADATA_ASSETS


def provenance_entries(dist: Path, tag: str, commit: str) -> list[dict[str, object]]:
    entries = []
    for target, name in sorted(DIRECT_ASSETS.items()):
        provenance_path = dist / f"{name}.provenance.json"
        data = json.loads(provenance_path.read_text(encoding="utf-8"))
        expected = {
            "schemaVersion": 1,
            "name": name,
            "targetTriple": target,
            "size": (dist / name).stat().st_size,
            "sha256": digest(dist / name),
            "sourceRepository": REPOSITORY,
            "sourceTag": tag,
            "sourceCommit": commit,
            "builderWorkflow": ".github/workflows/release.yml",
            "runnerEnvironment": "github-hosted",
        }
        if data != expected:
            raise ValueError(f"provenance identity mismatch for {name}")
        entries.append(data)
    return entries


def manifest_payload(dist: Path, tag: str, commit: str) -> dict[str, object]:
    return {
        "schemaVersion": 1,
        "repository": REPOSITORY,
        "sourceTag": tag,
        "sourceCommit": commit,
        "assets": provenance_entries(dist, tag, commit),
    }


def write_manifest(dist: Path, tag: str, commit: str) -> None:
    (dist / "v8-runner-assets.json").write_text(
        json.dumps(manifest_payload(dist, tag, commit), ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )


def verify_assets(dist: Path, tag: str, commit: str) -> None:
    actual = {path.name for path in dist.iterdir() if path.is_file()}
    expected = expected_release_files()
    if actual != expected:
        raise ValueError(f"release asset set mismatch: missing={sorted(expected - actual)}, extra={sorted(actual - expected)}")
    for name in sorted(set(DIRECT_ASSETS.values()) | ARCHIVE_ASSETS):
        path = dist / name
        checksum = parse_checksum((dist / f"{name}.sha256").read_text(encoding="utf-8"))
        if checksum != (digest(path), name):
            raise ValueError(f"checksum mismatch for {name}")
    manifest = json.loads((dist / "v8-runner-assets.json").read_text(encoding="utf-8"))
    if manifest != manifest_payload(dist, tag, commit):
        raise ValueError("release manifest source identity mismatch")


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser()
    commands = root.add_subparsers(dest="command", required=True)
    verify = commands.add_parser("verify-binary")
    verify.add_argument("--binary", required=True)
    verify.add_argument("--target", required=True)
    verify.add_argument("--version", required=True)
    direct = commands.add_parser("prepare-direct")
    for name in ("binary", "output", "target", "asset-name", "tag", "commit", "version"):
        direct.add_argument(f"--{name}", required=True)
    manifest = commands.add_parser("write-manifest")
    manifest.add_argument("--dist", required=True)
    manifest.add_argument("--tag", required=True)
    manifest.add_argument("--commit", required=True)
    check = commands.add_parser("verify-assets")
    check.add_argument("--dist", required=True)
    check.add_argument("--tag", required=True)
    check.add_argument("--commit", required=True)
    return root


def main() -> None:
    args = parser().parse_args()
    if args.command == "verify-binary":
        verify_binary(Path(args.binary), args.target, args.version)
    elif args.command == "prepare-direct":
        prepare_direct(args)
    elif args.command == "write-manifest":
        write_manifest(Path(args.dist), args.tag, args.commit)
    else:
        verify_assets(Path(args.dist), args.tag, args.commit)


if __name__ == "__main__":
    main()
