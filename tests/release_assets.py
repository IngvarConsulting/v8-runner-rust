#!/usr/bin/env python3
from __future__ import annotations

import importlib.util
import io
import json
import struct
import tarfile
import tempfile
import unittest
import zipfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
MODULE_PATH = ROOT / "scripts/release/release_assets.py"


def load_module():
    spec = importlib.util.spec_from_file_location("release_assets", MODULE_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError("cannot load release_assets")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class ReleaseAssetsTest(unittest.TestCase):
    def binary_for(self, target: str) -> bytes:
        if target == "x86_64-unknown-linux-musl":
            binary = bytearray(128)
            binary[:6] = b"\x7fELF\x02\x01"
            struct.pack_into("<H", binary, 18, 62)
            return bytes(binary)
        if target == "x86_64-pc-windows-msvc":
            binary = bytearray(256)
            binary[:2] = b"MZ"
            struct.pack_into("<I", binary, 0x3C, 128)
            binary[128:132] = b"PE\x00\x00"
            struct.pack_into("<H", binary, 132, 0x8664)
            return bytes(binary)
        cpu = {
            "x86_64-apple-darwin": 0x01000007,
            "aarch64-apple-darwin": 0x0100000C,
        }[target]
        binary = bytearray(b"\xcf\xfa\xed\xfe" + b"\x00" * 28)
        struct.pack_into("<I", binary, 4, cpu)
        return bytes(binary)

    def populate_release(self, module, dist: Path, windows_crlf: bool = False) -> None:
        binaries = {
            target: self.binary_for(target)
            for target in module.ARCHIVE_ASSETS
        }
        for target, name in module.DIRECT_ASSETS.items():
            (dist / name).write_bytes(binaries[target])
        (dist / "license-v8-runner-AGPL-3.0-only.txt").write_bytes(b"license\n")
        (dist / "notice-v8-runner-fork.txt").write_bytes(b"notice\n")

        for target, descriptor in module.ARCHIVE_ASSETS.items():
            archive_path = dist / descriptor["name"]
            files = {
                descriptor["binaryPath"]: binaries[target],
                f"{descriptor['root']}/README.md": (
                    module.SOURCE_ROOT / "README.md"
                ).read_bytes(),
                f"{descriptor['root']}/LICENSE": b"license\n",
                f"{descriptor['root']}/FORK_NOTICE.md": b"notice\n",
            }
            if windows_crlf and target == "x86_64-pc-windows-msvc":
                files[f"{descriptor['root']}/LICENSE"] = b"license\r\n"
                files[f"{descriptor['root']}/FORK_NOTICE.md"] = b"notice\r\n"
            if descriptor["format"] == "zip":
                with zipfile.ZipFile(archive_path, "w") as archive:
                    for name, payload in files.items():
                        archive.writestr(name, payload)
            else:
                with tarfile.open(archive_path, "w:gz") as archive:
                    for name, payload in files.items():
                        info = tarfile.TarInfo(name)
                        info.size = len(payload)
                        archive.addfile(info, io.BytesIO(payload))

        module.write_manifest(dist, "v0.7.0", "a" * 40)

    def test_public_release_has_exactly_ten_consolidated_assets(self) -> None:
        module = load_module()
        self.assertEqual(
            module.expected_release_files(),
            {
                "v8-runner-darwin-arm64",
                "v8-runner-linux-x64",
                "v8-runner-win-x64.exe",
                "v8-runner-linux-x86_64-musl.tar.gz",
                "v8-runner-macos-aarch64.tar.gz",
                "v8-runner-macos-x86_64.tar.gz",
                "v8-runner-windows-x86_64.zip",
                "license-v8-runner-AGPL-3.0-only.txt",
                "notice-v8-runner-fork.txt",
                "v8-runner-assets.json",
            },
        )
        self.assertEqual(
            module.attested_assets(),
            sorted(module.payload_asset_names() | {"v8-runner-assets.json"}),
        )

    def test_canonical_direct_asset_mapping_is_exact(self) -> None:
        module = load_module()
        self.assertEqual(
            module.DIRECT_ASSETS,
            {
                "aarch64-apple-darwin": "v8-runner-darwin-arm64",
                "x86_64-pc-windows-msvc": "v8-runner-win-x64.exe",
                "x86_64-unknown-linux-musl": "v8-runner-linux-x64",
            },
        )

    def test_linux_musl_rejects_dynamic_interpreter(self) -> None:
        module = load_module()
        elf = bytearray(128)
        elf[:6] = b"\x7fELF\x02\x01"
        struct.pack_into("<H", elf, 18, 62)
        struct.pack_into("<Q", elf, 32, 64)
        struct.pack_into("<H", elf, 54, 56)
        struct.pack_into("<H", elf, 56, 1)
        struct.pack_into("<I", elf, 64, 3)
        with self.assertRaisesRegex(ValueError, "PT_INTERP"):
            module.validate_binary_bytes(bytes(elf), "x86_64-unknown-linux-musl")

    def test_binary_headers_match_target_architecture(self) -> None:
        module = load_module()

        pe = bytearray(256)
        pe[:2] = b"MZ"
        struct.pack_into("<I", pe, 0x3C, 128)
        pe[128:132] = b"PE\x00\x00"
        struct.pack_into("<H", pe, 132, 0x8664)
        module.validate_binary_bytes(bytes(pe), "x86_64-pc-windows-msvc")

        macho = bytearray(b"\xcf\xfa\xed\xfe" + b"\x00" * 28)
        struct.pack_into("<I", macho, 4, 0x0100000C)
        module.validate_binary_bytes(bytes(macho), "aarch64-apple-darwin")

    def test_verify_assets_rejects_manifest_mismatch_without_rewriting_it(self) -> None:
        module = load_module()
        with tempfile.TemporaryDirectory() as temp_dir:
            dist = Path(temp_dir)
            self.populate_release(module, dist)
            manifest_path = dist / "v8-runner-assets.json"
            bad_manifest = {
                "schemaVersion": 2,
                "release": {},
                "assets": [],
            }
            original = json.dumps(bad_manifest) + "\n"
            manifest_path.write_text(original, encoding="utf-8")

            with self.assertRaisesRegex(ValueError, "manifest"):
                module.verify_assets(dist, "v0.7.0", "a" * 40)

            self.assertEqual(manifest_path.read_text(encoding="utf-8"), original)

    def test_manifest_covers_every_other_asset_and_archive_binary(self) -> None:
        module = load_module()
        with tempfile.TemporaryDirectory() as temp_dir:
            dist = Path(temp_dir)
            self.populate_release(module, dist)

            manifest = json.loads((dist / "v8-runner-assets.json").read_text(encoding="utf-8"))
            manifest_bytes = (dist / "v8-runner-assets.json").read_bytes()
            self.assertTrue(manifest_bytes.endswith(b"\n"))
            self.assertNotIn(b"\r\n", manifest_bytes)
            self.assertEqual(manifest["schemaVersion"], 2)
            self.assertEqual(len(manifest["assets"]), 9)
            self.assertEqual(
                {entry["name"] for entry in manifest["assets"]},
                module.expected_release_files() - {"v8-runner-assets.json"},
            )
            self.assertEqual(
                [entry["name"] for entry in manifest["assets"]],
                sorted(entry["name"] for entry in manifest["assets"]),
            )
            for entry in manifest["assets"]:
                asset = dist / entry["name"]
                self.assertEqual(entry["size"], asset.stat().st_size)
                self.assertEqual(entry["sha256"], module.digest(asset))

            linux_archive = next(
                entry
                for entry in manifest["assets"]
                if entry["name"] == "v8-runner-linux-x86_64-musl.tar.gz"
            )
            self.assertEqual(linux_archive["role"], "portable-archive")
            self.assertEqual(linux_archive["format"], "tar.gz")
            self.assertEqual(
                linux_archive["binary"]["sha256"],
                module.digest(dist / "v8-runner-linux-x64"),
            )
            self.assertTrue(linux_archive["buildAttestationRequired"])

            module.verify_assets(dist, "v0.7.0", "a" * 40)

    def test_verify_assets_rejects_archive_whose_binary_differs_from_direct_asset(self) -> None:
        module = load_module()
        with tempfile.TemporaryDirectory() as temp_dir:
            dist = Path(temp_dir)
            self.populate_release(module, dist)
            (dist / "v8-runner-linux-x64").write_bytes(
                self.binary_for("x86_64-unknown-linux-musl") + b"different"
            )

            with self.assertRaisesRegex(ValueError, "differs from direct asset"):
                module.verify_assets(dist, "v0.7.0", "a" * 40)

    def test_windows_archive_accepts_equivalent_crlf_legal_documents(self) -> None:
        module = load_module()
        with tempfile.TemporaryDirectory() as temp_dir:
            dist = Path(temp_dir)
            self.populate_release(module, dist, windows_crlf=True)

            module.verify_assets(dist, "v0.7.0", "a" * 40)

    def test_verify_assets_rejects_unsafe_zip_member(self) -> None:
        module = load_module()
        with tempfile.TemporaryDirectory() as temp_dir:
            dist = Path(temp_dir)
            self.populate_release(module, dist)
            archive_path = dist / "v8-runner-windows-x86_64.zip"
            with zipfile.ZipFile(archive_path, "a") as archive:
                archive.writestr("../outside.txt", b"unsafe")

            with self.assertRaisesRegex(ValueError, "unsafe archive member"):
                module.write_manifest(dist, "v0.7.0", "a" * 40)

    def test_verify_assets_rejects_tar_symlink(self) -> None:
        module = load_module()
        with tempfile.TemporaryDirectory() as temp_dir:
            dist = Path(temp_dir)
            self.populate_release(module, dist)
            archive_path = dist / "v8-runner-linux-x86_64-musl.tar.gz"
            with tarfile.open(archive_path, "r:gz") as archive:
                files = {
                    member.name: archive.extractfile(member).read()
                    for member in archive.getmembers()
                    if member.isfile()
                }
            with tarfile.open(archive_path, "w:gz") as archive:
                for name, payload in files.items():
                    info = tarfile.TarInfo(name)
                    info.size = len(payload)
                    archive.addfile(info, io.BytesIO(payload))
                link = tarfile.TarInfo("v8-runner-linux-x86_64-musl/examples/link")
                link.type = tarfile.SYMTYPE
                link.linkname = "../../outside"
                archive.addfile(link)

            with self.assertRaisesRegex(ValueError, "unsupported archive member"):
                module.write_manifest(dist, "v0.7.0", "a" * 40)

    def test_verify_assets_rejects_noncanonical_manifest_bytes(self) -> None:
        module = load_module()
        with tempfile.TemporaryDirectory() as temp_dir:
            dist = Path(temp_dir)
            self.populate_release(module, dist)
            manifest_path = dist / "v8-runner-assets.json"
            canonical = manifest_path.read_bytes()
            variants = {
                "duplicate key": canonical.replace(
                    b'{\n  "schemaVersion": 2,',
                    b'{\n  "schemaVersion": 999,\n  "schemaVersion": 2,',
                    1,
                ),
                "CRLF": canonical.replace(b"\n", b"\r\n"),
                "leading whitespace": b" " + canonical,
                "different key order": canonical.replace(
                    b'{\n  "schemaVersion": 2,\n  "release":',
                    b'{\n  "release":',
                    1,
                ).replace(
                    b'  },\n  "assets":',
                    b'  },\n  "schemaVersion": 2,\n  "assets":',
                    1,
                ),
            }

            for label, variant in variants.items():
                with self.subTest(label=label):
                    manifest_path.write_bytes(variant)
                    with self.assertRaisesRegex(ValueError, "manifest"):
                        module.verify_assets(dist, "v0.7.0", "a" * 40)


if __name__ == "__main__":
    unittest.main()
