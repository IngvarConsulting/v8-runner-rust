#!/usr/bin/env python3
"""Create verified immutable archival mirrors of upstream releases."""

from __future__ import annotations

import hashlib
import json
import re
import subprocess
import tempfile
from pathlib import Path
from typing import Any

SOURCE = "alkoleft/v8-runner-rust"
TARGET = "IngvarConsulting/v8-runner-rust"
REPO = Path("/tmp/v8runner-capabilities.Alw0Ca/v8-runner-rust")
MANIFEST = Path("/tmp/v8runner-release-mirror-manifest.json")
LICENSE_INTRO_COMMIT = "d2427b12acbb50af1a01071c490720e89d2d4011"


def run(args: list[str], *, cwd: Path | None = None, binary: bool = False) -> str | bytes:
    completed = subprocess.run(args, cwd=cwd, stdout=subprocess.PIPE,
                               stderr=subprocess.PIPE, check=False)
    if completed.returncode != 0:
        raise RuntimeError(
            f"command failed ({completed.returncode}): {' '.join(args)}\n"
            f"{completed.stderr.decode(errors='replace')}"
        )
    return completed.stdout if binary else completed.stdout.decode()


def api(endpoint: str, *, method: str = "GET",
        payload: dict[str, Any] | None = None) -> Any:
    args = ["gh", "api", endpoint]
    if method != "GET":
        args += ["--method", method]
    if payload is not None:
        args += ["--input", "-"]
        completed = subprocess.run(args, input=json.dumps(payload).encode(),
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        if completed.returncode != 0:
            raise RuntimeError(completed.stderr.decode(errors="replace"))
        output = completed.stdout.decode()
    else:
        output = run(args)
    return json.loads(output) if output.strip() else None


def api_list(endpoint: str) -> list[dict[str, Any]]:
    return json.loads(run(["gh", "api", "--paginate", endpoint]))


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def sanitize(value: str | None) -> str:
    text = (value or "").replace("@", "@\u200b")
    return re.sub(
        r"(?i)\b(close[sd]?|fix(?:e[sd])?|resolve[sd]?)\s+#(\d+)",
        lambda match: f"{match.group(1)} upstream #{match.group(2)}",
        text,
    ).strip()


def ref_sha(repo: str, tag: str) -> str:
    return api(f"repos/{repo}/git/ref/tags/{tag}")["object"]["sha"]


def main() -> None:
    releases = api_list(f"repos/{SOURCE}/releases?per_page=100")
    releases.sort(key=lambda release: release["published_at"] or release["created_at"])
    target_existing = api_list(f"repos/{TARGET}/releases?per_page=100")
    if target_existing:
        raise RuntimeError("target already has releases; refusing ambiguous mirror run")

    root = Path(tempfile.mkdtemp(prefix="v8runner-release-mirror."))
    records: list[dict[str, Any]] = []

    # Phase 1: download and verify every upstream byte before writing releases.
    for release in releases:
        tag = release["tag_name"]
        directory = root / tag
        directory.mkdir()
        print(f"download {tag}", flush=True)
        run(["gh", "release", "download", tag, "--repo", SOURCE, "--dir", str(directory)])
        source_ref = ref_sha(SOURCE, tag)
        target_ref = ref_sha(TARGET, tag)
        if source_ref != target_ref:
            raise RuntimeError(f"tag ref mismatch for {tag}: {source_ref} != {target_ref}")
        assets: list[dict[str, Any]] = []
        upstream_by_name = {asset["name"]: asset for asset in release["assets"]}
        if set(upstream_by_name) != {path.name for path in directory.iterdir()}:
            raise RuntimeError(f"asset name set mismatch for {tag}")
        for name, asset in sorted(upstream_by_name.items()):
            path = directory / name
            actual = sha256(path)
            expected = (asset.get("digest") or "").removeprefix("sha256:")
            if path.stat().st_size != asset["size"] or actual != expected:
                raise RuntimeError(f"asset mismatch for {tag}/{name}")
            assets.append({
                "source_asset_id": asset["id"], "name": name,
                "size": asset["size"], "sha256": actual,
                "source_uploader": (asset.get("uploader") or {}).get("login"),
                "source_created_at": asset.get("created_at"),
                "source_updated_at": asset.get("updated_at"),
            })
        for sidecar in directory.glob("*.sha256"):
            named = sidecar.name.removesuffix(".sha256")
            expected = sidecar.read_text(encoding="utf-8").split()[0]
            if expected != sha256(directory / named):
                raise RuntimeError(f"sidecar mismatch for {tag}/{sidecar.name}")
        license_path = directory / "LICENSE"
        tag_files = run(["git", "ls-tree", "-r", "--name-only", tag], cwd=REPO).splitlines()
        license_source = tag if "LICENSE" in tag_files else LICENSE_INTRO_COMMIT
        license_path.write_bytes(
            run(["git", "show", f"{license_source}:LICENSE"], cwd=REPO, binary=True)
        )
        records.append({
            "tag": tag, "source_release_id": release["id"],
            "source_html_url": release["html_url"],
            "source_author": (release.get("author") or {}).get("login"),
            "source_created_at": release["created_at"],
            "source_published_at": release["published_at"],
            "source_tag_ref_sha": source_ref,
            "target_tag_ref_sha": target_ref,
            "prerelease": release["prerelease"],
            "assets": assets,
            "target_extra_license_sha256": sha256(license_path),
            "license_source": license_source,
            "directory": str(directory),
            "source_body": release.get("body") or "",
            "source_name": release.get("name") or tag,
        })

    # Phase 2: create all drafts and upload verified assets plus the license.
    for record in records:
        tag = record["tag"]
        body = "\n".join([
            "## Verified archival mirror",
            "",
            f"This release mirrors [{SOURCE} {tag}]({record['source_html_url']}).",
            f"Original publisher: `{record['source_author']}`; original publication: "
            f"`{record['source_published_at']}`.",
            "All original assets were downloaded and verified against upstream API size and "
            "SHA-256 digest before upload. GitHub attributes this mirror and its publication "
            "time to Ingvar Consulting; it is not the original release.",
            f"Corresponding source and build scripts: "
            f"https://github.com/{TARGET}/tree/{tag}",
            (
                "The separately attached `LICENSE` is the AGPL-3.0 license from that exact tag; "
                "it was not present in the historical upstream archives."
                if record["license_source"] == tag
                else f"The tag predates the repository's LICENSE file. The separately attached "
                f"AGPL-3.0 `LICENSE` comes from upstream commit "
                f"`{record['license_source']}`, which added only that file after this release. "
                "This note preserves the historical licensing gap rather than pretending the "
                "file existed in the tag."
            ),
            "",
            "---",
            "",
            sanitize(record["source_body"]),
        ]).rstrip()
        created = api(f"repos/{TARGET}/releases", method="POST", payload={
            "tag_name": tag, "name": record["source_name"], "body": body,
            "draft": True, "prerelease": record["prerelease"],
        })
        record["target_release_id"] = created["id"]
        upload_paths = [str(path) for path in sorted(Path(record["directory"]).iterdir())]
        run(["gh", "release", "upload", tag, *upload_paths, "--repo", TARGET])
        print(f"draft {tag}: uploaded {len(upload_paths)} files", flush=True)

    # Phase 3: verify target drafts byte-for-byte, then publish oldest to newest.
    for record in records:
        release = api(f"repos/{TARGET}/releases/{record['target_release_id']}")
        target_assets = {asset["name"]: asset for asset in release["assets"]}
        expected = {asset["name"]: asset for asset in record["assets"]}
        expected["LICENSE"] = {
            "name": "LICENSE", "sha256": record["target_extra_license_sha256"],
            "size": (Path(record["directory"]) / "LICENSE").stat().st_size,
        }
        if set(target_assets) != set(expected):
            raise RuntimeError(f"target asset set mismatch for {record['tag']}")
        for name, wanted in expected.items():
            got = target_assets[name]
            digest = (got.get("digest") or "").removeprefix("sha256:")
            if got["size"] != wanted["size"] or digest != wanted["sha256"]:
                raise RuntimeError(f"target digest mismatch for {record['tag']}/{name}")
        published = api(f"repos/{TARGET}/releases/{record['target_release_id']}",
                        method="PATCH", payload={"draft": False})
        record["target_html_url"] = published["html_url"]
        record["target_published_at"] = published["published_at"]
        print(f"published immutable {record['tag']}", flush=True)

    for record in records:
        record.pop("directory", None)
        record.pop("source_body", None)
    MANIFEST.write_text(json.dumps({
        "source": SOURCE, "target": TARGET,
        "verified_archival_mirrors": records,
    }, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"manifest: {MANIFEST}")


if __name__ == "__main__":
    main()
