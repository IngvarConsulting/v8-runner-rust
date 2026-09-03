#!/usr/bin/env python3
"""Build and verify the fork-owned consolidated release asset contract."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import stat
import struct
import subprocess
import tarfile
import zipfile
from pathlib import Path, PurePosixPath


REPOSITORY = "https://github.com/IngvarConsulting/v8-runner-rust"
SOURCE_ROOT = Path(__file__).resolve().parents[2]
BUILDER_WORKFLOW = ".github/workflows/release.yml"
MANIFEST_ASSET = "v8-runner-assets.json"
DIRECT_ASSETS = {
    "aarch64-apple-darwin": "v8-runner-darwin-arm64",
    "x86_64-pc-windows-msvc": "v8-runner-win-x64.exe",
    "x86_64-unknown-linux-musl": "v8-runner-linux-x64",
}
ARCHIVE_ASSETS = {
    "x86_64-unknown-linux-musl": {
        "name": "v8-runner-linux-x86_64-musl.tar.gz",
        "root": "v8-runner-linux-x86_64-musl",
        "binaryPath": "v8-runner-linux-x86_64-musl/v8-runner",
        "format": "tar.gz",
    },
    "aarch64-apple-darwin": {
        "name": "v8-runner-macos-aarch64.tar.gz",
        "root": "v8-runner-macos-aarch64",
        "binaryPath": "v8-runner-macos-aarch64/v8-runner",
        "format": "tar.gz",
    },
    "x86_64-apple-darwin": {
        "name": "v8-runner-macos-x86_64.tar.gz",
        "root": "v8-runner-macos-x86_64",
        "binaryPath": "v8-runner-macos-x86_64/v8-runner",
        "format": "tar.gz",
    },
    "x86_64-pc-windows-msvc": {
        "name": "v8-runner-windows-x86_64.zip",
        "root": "v8-runner-windows-x86_64",
        "binaryPath": "v8-runner-windows-x86_64/v8-runner.exe",
        "format": "zip",
    },
}
DOCUMENT_ASSETS = {
    "license-v8-runner-AGPL-3.0-only.txt": "license",
    "notice-v8-runner-fork.txt": "fork-notice",
}


def digest(path: Path) -> str:
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(chunk)
    return value.hexdigest()


def validate_binary_bytes(data: bytes, target: str) -> None:
    if target == "x86_64-unknown-linux-musl":
        if (
            len(data) < 64
            or data[:6] != b"\x7fELF\x02\x01"
            or struct.unpack_from("<H", data, 18)[0] != 62
        ):
            raise ValueError("expected little-endian ELF64 x86-64")
        phoff = struct.unpack_from("<Q", data, 32)[0]
        phentsize = struct.unpack_from("<H", data, 54)[0]
        phnum = struct.unpack_from("<H", data, 56)[0]
        if phnum and phentsize < 4:
            raise ValueError("ELF program header entry is truncated")
        if phoff + phentsize * phnum > len(data):
            raise ValueError("ELF program headers are truncated")
        if any(
            struct.unpack_from("<I", data, phoff + index * phentsize)[0] == 3
            for index in range(phnum)
        ):
            raise ValueError("musl release must not contain PT_INTERP")
        return

    if target == "x86_64-pc-windows-msvc":
        if len(data) < 0x40 or data[:2] != b"MZ":
            raise ValueError("expected PE executable")
        offset = struct.unpack_from("<I", data, 0x3C)[0]
        if (
            offset + 6 > len(data)
            or data[offset : offset + 4] != b"PE\x00\x00"
            or struct.unpack_from("<H", data, offset + 4)[0] != 0x8664
        ):
            raise ValueError("expected PE x86-64")
        return

    expected_cpu = {
        "x86_64-apple-darwin": 0x01000007,
        "aarch64-apple-darwin": 0x0100000C,
    }.get(target)
    if expected_cpu is not None:
        if (
            data[:4] != b"\xcf\xfa\xed\xfe"
            or struct.unpack_from("<I", data, 4)[0] != expected_cpu
        ):
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
        raise ValueError(
            f"{path.name} --version did not report {version!r}: {completed.stdout!r}"
        )


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


def payload_asset_names() -> set[str]:
    return set(DIRECT_ASSETS.values()) | {
        descriptor["name"] for descriptor in ARCHIVE_ASSETS.values()
    }


def expected_release_files() -> set[str]:
    return payload_asset_names() | set(DOCUMENT_ASSETS) | {MANIFEST_ASSET}


def _validated_member_name(
    path: Path,
    name: str,
    descriptor: dict[str, str],
    *,
    is_directory: bool,
) -> str:
    trimmed = name[:-1] if is_directory and name.endswith("/") else name
    parts = trimmed.split("/")
    if (
        not trimmed
        or "\\" in trimmed
        or PurePosixPath(trimmed).is_absolute()
        or any(part in {"", ".", ".."} for part in parts)
        or parts[0] != descriptor["root"]
    ):
        raise ValueError(f"unsafe archive member in {path.name}: {name!r}")

    canonical = "/".join(parts)
    required_files = {
        descriptor["binaryPath"],
        f"{descriptor['root']}/README.md",
        f"{descriptor['root']}/LICENSE",
        f"{descriptor['root']}/FORK_NOTICE.md",
    }
    under_examples = (
        len(parts) >= 2
        and parts[1] == "examples"
        and (is_directory or len(parts) >= 3)
    )
    if is_directory:
        allowed = canonical == descriptor["root"] or under_examples
    else:
        allowed = canonical in required_files or under_examples
    if not allowed:
        raise ValueError(f"unexpected archive member in {path.name}: {name!r}")
    return canonical


def _archive_members(
    path: Path, descriptor: dict[str, str]
) -> dict[str, bytes]:
    format_ = descriptor["format"]
    result: dict[str, bytes] = {}
    seen: set[str] = set()

    def register(name: str, *, is_directory: bool) -> str:
        canonical = _validated_member_name(
            path, name, descriptor, is_directory=is_directory
        )
        identity = canonical.casefold()
        if identity in seen:
            raise ValueError(f"duplicate archive member in {path.name}: {name!r}")
        seen.add(identity)
        return canonical

    if format_ == "zip":
        with zipfile.ZipFile(path) as archive:
            for member in archive.infolist():
                mode = (member.external_attr >> 16) & 0xFFFF
                file_type = stat.S_IFMT(mode)
                is_directory = member.is_dir()
                if file_type == stat.S_IFLNK:
                    raise ValueError(
                        f"unsupported archive member in {path.name}: {member.filename!r}"
                    )
                expected_types = {0, stat.S_IFDIR if is_directory else stat.S_IFREG}
                if file_type not in expected_types:
                    raise ValueError(
                        f"unsupported archive member in {path.name}: {member.filename!r}"
                    )
                canonical = register(member.filename, is_directory=is_directory)
                if not is_directory:
                    result[canonical] = archive.read(member)
            return result
    if format_ == "tar.gz":
        with tarfile.open(path, "r:gz") as archive:
            for member in archive.getmembers():
                if not member.isfile() and not member.isdir():
                    raise ValueError(
                        f"unsupported archive member in {path.name}: {member.name!r}"
                    )
                canonical = register(member.name, is_directory=member.isdir())
                if member.isfile():
                    stream = archive.extractfile(member)
                    if stream is None:
                        raise ValueError(f"cannot read {member.name} from {path.name}")
                    result[canonical] = stream.read()
            return result
    raise ValueError(f"unsupported archive format: {format_}")


def _canonical_text(data: bytes) -> bytes:
    return data.replace(b"\r\n", b"\n").replace(b"\r", b"\n")


def _archive_binary(dist: Path, target: str, descriptor: dict[str, str]) -> bytes:
    path = dist / descriptor["name"]
    members = _archive_members(path, descriptor)
    required = {
        descriptor["binaryPath"],
        f"{descriptor['root']}/README.md",
        f"{descriptor['root']}/LICENSE",
        f"{descriptor['root']}/FORK_NOTICE.md",
    }
    missing = required - set(members)
    if missing:
        raise ValueError(f"{path.name} is missing required files: {sorted(missing)}")
    if _canonical_text(
        members[f"{descriptor['root']}/README.md"]
    ) != _canonical_text((SOURCE_ROOT / "README.md").read_bytes()):
        raise ValueError(f"{path.name} contains a different README.md")
    if _canonical_text(members[f"{descriptor['root']}/LICENSE"]) != _canonical_text(
        (dist / "license-v8-runner-AGPL-3.0-only.txt").read_bytes()
    ):
        raise ValueError(f"{path.name} contains a different LICENSE")
    if _canonical_text(
        members[f"{descriptor['root']}/FORK_NOTICE.md"]
    ) != _canonical_text((dist / "notice-v8-runner-fork.txt").read_bytes()):
        raise ValueError(f"{path.name} contains a different FORK_NOTICE.md")
    binary = members[descriptor["binaryPath"]]
    validate_binary_bytes(binary, target)
    direct_name = DIRECT_ASSETS.get(target)
    if direct_name is not None and binary != (dist / direct_name).read_bytes():
        raise ValueError(f"{path.name} binary differs from direct asset {direct_name}")
    return binary


def manifest_entries(dist: Path) -> list[dict[str, object]]:
    entries: list[dict[str, object]] = []
    for target, name in sorted(DIRECT_ASSETS.items()):
        path = dist / name
        validate_binary_bytes(path.read_bytes(), target)
        entries.append(
            {
                "name": name,
                "role": "direct-binary",
                "targetTriple": target,
                "format": "executable",
                "size": path.stat().st_size,
                "sha256": digest(path),
                "buildAttestationRequired": True,
            }
        )
    for target, descriptor in sorted(ARCHIVE_ASSETS.items()):
        path = dist / descriptor["name"]
        binary = _archive_binary(dist, target, descriptor)
        entries.append(
            {
                "name": descriptor["name"],
                "role": "portable-archive",
                "targetTriple": target,
                "format": descriptor["format"],
                "size": path.stat().st_size,
                "sha256": digest(path),
                "buildAttestationRequired": True,
                "binary": {
                    "path": descriptor["binaryPath"],
                    "size": len(binary),
                    "sha256": hashlib.sha256(binary).hexdigest(),
                },
            }
        )
    for name, role in sorted(DOCUMENT_ASSETS.items()):
        path = dist / name
        entries.append(
            {
                "name": name,
                "role": role,
                "format": "text/plain",
                "size": path.stat().st_size,
                "sha256": digest(path),
                "buildAttestationRequired": False,
            }
        )
    return sorted(entries, key=lambda entry: str(entry["name"]))


def manifest_payload(dist: Path, tag: str, commit: str) -> dict[str, object]:
    if not tag.startswith("v"):
        raise ValueError(f"release tag must start with v: {tag}")
    if len(commit) != 40 or any(
        character not in "0123456789abcdef" for character in commit
    ):
        raise ValueError(
            f"source commit must be 40 lowercase hexadecimal characters: {commit}"
        )
    return {
        "schemaVersion": 2,
        "release": {
            "repository": REPOSITORY,
            "sourceTag": tag,
            "sourceCommit": commit,
            "builderWorkflow": BUILDER_WORKFLOW,
            "runnerEnvironment": "github-hosted",
        },
        "assets": manifest_entries(dist),
    }


def canonical_manifest_bytes(dist: Path, tag: str, commit: str) -> bytes:
    return (
        json.dumps(
            manifest_payload(dist, tag, commit),
            ensure_ascii=False,
            allow_nan=False,
            indent=2,
        )
        + "\n"
    ).encode("utf-8")


def write_manifest(dist: Path, tag: str, commit: str) -> None:
    (dist / MANIFEST_ASSET).write_bytes(canonical_manifest_bytes(dist, tag, commit))


def _reject_duplicate_json_keys(pairs: list[tuple[str, object]]) -> dict[str, object]:
    result: dict[str, object] = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"release manifest contains duplicate key: {key!r}")
        result[key] = value
    return result


def verify_assets(dist: Path, tag: str, commit: str) -> None:
    actual = {path.name for path in dist.iterdir() if path.is_file()}
    expected = expected_release_files()
    if actual != expected:
        raise ValueError(
            f"release asset set mismatch: missing={sorted(expected - actual)}, "
            f"extra={sorted(actual - expected)}"
        )
    manifest_bytes = (dist / MANIFEST_ASSET).read_bytes()
    try:
        json.loads(
            manifest_bytes.decode("utf-8"),
            object_pairs_hook=_reject_duplicate_json_keys,
        )
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError(f"release manifest is not canonical UTF-8 JSON: {error}") from error
    if manifest_bytes != canonical_manifest_bytes(dist, tag, commit):
        raise ValueError(
            "release manifest bytes do not match the canonical asset set and source identity"
        )


def attested_assets() -> list[str]:
    return sorted(payload_asset_names() | {MANIFEST_ASSET})


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser()
    commands = root.add_subparsers(dest="command", required=True)
    verify = commands.add_parser("verify-binary")
    verify.add_argument("--binary", required=True)
    verify.add_argument("--target", required=True)
    verify.add_argument("--version", required=True)
    direct = commands.add_parser("prepare-direct")
    for name in ("binary", "output", "target", "asset-name", "version"):
        direct.add_argument(f"--{name}", required=True)
    manifest = commands.add_parser("write-manifest")
    manifest.add_argument("--dist", required=True)
    manifest.add_argument("--tag", required=True)
    manifest.add_argument("--commit", required=True)
    check = commands.add_parser("verify-assets")
    check.add_argument("--dist", required=True)
    check.add_argument("--tag", required=True)
    check.add_argument("--commit", required=True)
    commands.add_parser("attested-assets")
    return root


def main() -> None:
    args = parser().parse_args()
    if args.command == "verify-binary":
        verify_binary(Path(args.binary), args.target, args.version)
    elif args.command == "prepare-direct":
        prepare_direct(args)
    elif args.command == "write-manifest":
        write_manifest(Path(args.dist), args.tag, args.commit)
    elif args.command == "verify-assets":
        verify_assets(Path(args.dist), args.tag, args.commit)
    else:
        print("\n".join(attested_assets()))


if __name__ == "__main__":
    main()
