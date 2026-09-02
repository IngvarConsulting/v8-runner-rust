#!/usr/bin/env python3
from __future__ import annotations

import importlib.util
import json
import struct
import tempfile
import unittest
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
            for name in module.expected_release_files():
                (dist / name).write_bytes(b"payload")
            for name in set(module.DIRECT_ASSETS.values()) | module.ARCHIVE_ASSETS:
                (dist / f"{name}.sha256").write_text(
                    f"{module.digest(dist / name)}  {name}\n", encoding="utf-8"
                )
            for target, name in module.DIRECT_ASSETS.items():
                provenance = {
                    "schemaVersion": 1,
                    "name": name,
                    "targetTriple": target,
                    "size": (dist / name).stat().st_size,
                    "sha256": module.digest(dist / name),
                    "sourceRepository": module.REPOSITORY,
                    "sourceTag": "v0.5.2-ic.2",
                    "sourceCommit": "a" * 40,
                    "builderWorkflow": ".github/workflows/release.yml",
                    "runnerEnvironment": "github-hosted",
                }
                (dist / f"{name}.provenance.json").write_text(
                    json.dumps(provenance) + "\n", encoding="utf-8"
                )
            manifest_path = dist / "v8-runner-assets.json"
            bad_manifest = {
                "schemaVersion": 1,
                "repository": module.REPOSITORY,
                "sourceTag": "v-wrong",
                "sourceCommit": "wrong",
                "assets": [],
            }
            original = json.dumps(bad_manifest) + "\n"
            manifest_path.write_text(original, encoding="utf-8")

            with self.assertRaisesRegex(ValueError, "manifest"):
                module.verify_assets(dist, "v0.5.2-ic.2", "a" * 40)

            self.assertEqual(manifest_path.read_text(encoding="utf-8"), original)


if __name__ == "__main__":
    unittest.main()
