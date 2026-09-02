#!/usr/bin/env python3
"""Mirror the upstream v8-runner tracker without changing trust semantics."""

from __future__ import annotations

import json
import re
import subprocess
import sys
from pathlib import Path
from typing import Any
from urllib.parse import quote

SOURCE = "alkoleft/v8-runner-rust"
TARGET = "IngvarConsulting/v8-runner-rust"
REPO = Path("/tmp/v8runner-capabilities.Alw0Ca/v8-runner-rust")
MAP_PATH = Path("/tmp/v8runner-tracker-map.json")
SNAPSHOT_PATH = Path("/tmp/v8runner-upstream-tracker-snapshot.json")
MIGRATED_LABEL = "upstream-migrated"
ARCHIVE_LABEL = "upstream-pr-archive"


def run(args: list[str], *, cwd: Path | None = None, stdin: str | None = None) -> str:
    completed = subprocess.run(
        args, cwd=cwd, input=stdin, text=True, stdout=subprocess.PIPE,
        stderr=subprocess.PIPE, check=False,
    )
    if completed.returncode != 0:
        raise RuntimeError(
            f"command failed ({completed.returncode}): {' '.join(args)}\n{completed.stderr}"
        )
    return completed.stdout


def api(endpoint: str, *, method: str = "GET", payload: dict[str, Any] | None = None) -> Any:
    args = ["gh", "api", endpoint]
    if method != "GET":
        args.extend(["--method", method])
    stdin = None
    if payload is not None:
        args.extend(["--input", "-"])
        stdin = json.dumps(payload, ensure_ascii=False)
    output = run(args, stdin=stdin)
    return json.loads(output) if output.strip() else None


def api_list(endpoint: str) -> list[dict[str, Any]]:
    output = run(["gh", "api", "--paginate", "--slurp", endpoint])
    return [item for page in json.loads(output) for item in page]


def actor_login(item: dict[str, Any]) -> str:
    return (item.get("user") or {}).get("login") or "ghost"


def issue_marker(kind: str, number: int) -> str:
    return f"<!-- upstream-migration:{SOURCE}:{kind}:{number} -->"


def comment_marker(kind: str, comment_id: int) -> str:
    return f"<!-- upstream-{kind}:{SOURCE}:{comment_id} -->"


def sanitize_markdown(value: str | None) -> str:
    """Preserve prose without notifications or target-issue state changes."""
    text = (value or "").replace("@", "@\u200b")
    text = re.sub(
        r"(?i)\b(close[sd]?|fix(?:e[sd])?|resolve[sd]?)\s+#(\d+)",
        lambda match: f"{match.group(1)} upstream #{match.group(2)}",
        text,
    )
    return text.strip()


def attribution(item: dict[str, Any], *, kind: str) -> str:
    number = item["number"]
    author = actor_login(item)
    return "\n".join([
        issue_marker(kind, number),
        f"> Migrated from [{SOURCE} {kind} #{number}]({item['html_url']}).",
        f"> Original author: [{author}](https://github.com/{author}); created: "
        f"`{item['created_at']}`; last upstream update: `{item['updated_at']}`.",
        "> GitHub shows the migration account as the new author; this block preserves actual attribution.",
    ])


def migrated_body(item: dict[str, Any], *, kind: str) -> str:
    body = sanitize_markdown(item.get("body"))
    metadata = attribution(item, kind=kind)
    if kind == "pr":
        metadata += (
            f"\n> Upstream historical base: `{item['base']['ref']}` at `{item['base']['sha']}`; "
            f"head: `{item['head']['label']}` at `{item['head']['sha']}`. Active migrated PRs "
            f"compare that exact head with the current `{TARGET}:master`."
        )
        if item.get("merged_at"):
            metadata += (
                f"\n> Upstream merged: `{item['merged_at']}`; "
                f"merge commit: `{item.get('merge_commit_sha')}`."
            )
    return f"{metadata}\n\n---\n\n{body}" if body else metadata


def quoted_comment(comment: dict[str, Any]) -> str:
    author = actor_login(comment)
    body = sanitize_markdown(comment.get("body"))
    return (
        f"{comment_marker('comment', comment['id'])}\n"
        f"> Upstream comment by [{author}](https://github.com/{author}) at "
        f"`{comment['created_at']}`: [original]({comment['html_url']})\n\n{body}"
    )


def quoted_review(review: dict[str, Any]) -> str:
    author = actor_login(review)
    details = (
        f"{comment_marker('review', review['id'])}\n"
        f"> Upstream review by [{author}](https://github.com/{author}); "
        f"state: `{review['state']}`; submitted: `{review.get('submitted_at')}`; "
        f"commit: `{review.get('commit_id')}`. [Original review]({review['html_url']})"
    )
    body = sanitize_markdown(review.get("body"))
    return f"{details}\n\n{body}" if body else details


def quoted_review_comment(comment: dict[str, Any]) -> str:
    author = actor_login(comment)
    location = f"path: `{comment.get('path')}`"
    if comment.get("line") is not None:
        location += f", line: `{comment['line']}`"
    elif comment.get("original_line") is not None:
        location += f", original line: `{comment['original_line']}`"
    details = (
        f"{comment_marker('review-comment', comment['id'])}\n"
        f"> Upstream inline review comment by [{author}](https://github.com/{author}) "
        f"at `{comment['created_at']}`; {location}. [Original comment]({comment['html_url']})"
    )
    body = sanitize_markdown(comment.get("body"))
    return f"{details}\n\n{body}" if body else details


def assert_actions_disabled() -> None:
    if api(f"repos/{TARGET}/actions/permissions").get("enabled") is not False:
        raise RuntimeError("target Actions must be disabled before tracker migration")


def ensure_labels() -> None:
    source_labels = api_list(f"repos/{SOURCE}/labels?per_page=100")
    target_labels = {
        label["name"]: label for label in api_list(f"repos/{TARGET}/labels?per_page=100")
    }
    desired = list(source_labels) + [
        {"name": MIGRATED_LABEL, "color": "5319e7", "description": "Migrated from the historical upstream tracker"},
        {"name": ARCHIVE_LABEL, "color": "6f42c1", "description": "Closed or merged upstream pull request preserved as an archive"},
    ]
    for label in desired:
        if label["name"] not in target_labels:
            api(f"repos/{TARGET}/labels", method="POST", payload={
                "name": label["name"], "color": label["color"],
                "description": label.get("description") or "",
            })


def target_items_by_marker() -> dict[str, dict[str, Any]]:
    found: dict[str, dict[str, Any]] = {}
    for item in api_list(f"repos/{TARGET}/issues?state=all&per_page=100"):
        match = re.search(
            rf"<!-- upstream-migration:{re.escape(SOURCE)}:(issue|pr):(\d+) -->",
            item.get("body") or "",
        )
        if match:
            found[f"{match.group(1)}:{match.group(2)}"] = item
    return found


def ensure_comments(target_number: int, source_number: int, *, include_reviews: bool) -> None:
    target_comments = api_list(f"repos/{TARGET}/issues/{target_number}/comments?per_page=100")
    seen = "\n".join(comment.get("body") or "" for comment in target_comments)
    for comment in api_list(f"repos/{SOURCE}/issues/{source_number}/comments?per_page=100"):
        marker = comment_marker("comment", comment["id"])
        if marker not in seen:
            api(f"repos/{TARGET}/issues/{target_number}/comments", method="POST",
                payload={"body": quoted_comment(comment)})
    if not include_reviews:
        return
    for review in api_list(f"repos/{SOURCE}/pulls/{source_number}/reviews?per_page=100"):
        marker = comment_marker("review", review["id"])
        if marker not in seen:
            api(f"repos/{TARGET}/issues/{target_number}/comments", method="POST",
                payload={"body": quoted_review(review)})
    for comment in api_list(f"repos/{SOURCE}/pulls/{source_number}/comments?per_page=100"):
        marker = comment_marker("review-comment", comment["id"])
        if marker not in seen:
            api(f"repos/{TARGET}/issues/{target_number}/comments", method="POST",
                payload={"body": quoted_review_comment(comment)})


def close_issue(number: int, source: dict[str, Any]) -> None:
    reason = source.get("state_reason")
    if reason not in {"completed", "not_planned"}:
        reason = "completed"
    api(f"repos/{TARGET}/issues/{number}", method="PATCH",
        payload={"state": "closed", "state_reason": reason})


def create_issue(source: dict[str, Any], *, archive_pr: bool) -> dict[str, Any]:
    labels = [label["name"] for label in source.get("labels", [])] + [MIGRATED_LABEL]
    if archive_pr:
        labels.append(ARCHIVE_LABEL)
    created = api(f"repos/{TARGET}/issues", method="POST", payload={
        "title": source["title"],
        "body": migrated_body(source, kind="pr" if archive_pr else "issue"),
        "labels": sorted(set(labels)),
    })
    ensure_comments(created["number"], source["number"], include_reviews=archive_pr)
    if source["state"] == "closed" or archive_pr:
        close_issue(created["number"], source)
    return created


def push_quarantine_branch(source: dict[str, Any]) -> tuple[str, str]:
    number = source["number"]
    local_ref = f"refs/remotes/origin/migration-pr-{number}"
    run(["git", "fetch", "origin", f"+refs/pull/{number}/head:{local_ref}"], cwd=REPO)
    sha = run(["git", "rev-parse", local_ref], cwd=REPO).strip()
    if sha != source["head"]["sha"]:
        raise RuntimeError(f"PR #{number} head mismatch: API={source['head']['sha']} fetched={sha}")
    branch = f"quarantine/upstream-pr-{number}"
    remote_ref = f"refs/remotes/org/{branch}"
    run(["git", "fetch", "org", "+refs/heads/*:refs/remotes/org/*"], cwd=REPO)
    existing = subprocess.run(
        ["git", "rev-parse", "--verify", "--quiet", remote_ref], cwd=REPO,
        text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False,
    )
    if existing.returncode == 0:
        if existing.stdout.strip() != sha:
            raise RuntimeError(
                f"quarantine branch {branch} moved: expected={sha} actual={existing.stdout.strip()}"
            )
    else:
        run(["git", "push", "org", f"{sha}:refs/heads/{branch}"], cwd=REPO)
    return branch, sha


def lock_quarantine_branch(branch: str) -> None:
    api(f"repos/{TARGET}/branches/{quote(branch, safe='')}/protection", method="PUT", payload={
        "required_status_checks": None, "enforce_admins": True,
        "required_pull_request_reviews": None, "restrictions": None,
        "required_linear_history": False, "allow_force_pushes": False,
        "allow_deletions": False, "block_creations": False,
        "required_conversation_resolution": False, "lock_branch": True,
        "allow_fork_syncing": False,
    })


def create_open_pr(source: dict[str, Any]) -> dict[str, Any]:
    branch, sha = push_quarantine_branch(source)
    lock_quarantine_branch(branch)
    labels = [label["name"] for label in source.get("labels", [])] + [MIGRATED_LABEL]
    created = api(f"repos/{TARGET}/pulls", method="POST", payload={
        "title": source["title"], "body": migrated_body(source, kind="pr"),
        "head": source["head"]["label"], "base": "master",
        "maintainer_can_modify": False,
    })
    if created["head"]["sha"] != sha:
        raise RuntimeError(
            f"PR #{source['number']} target head mismatch: expected={sha} actual={created['head']['sha']}"
        )
    if created["head"]["repo"]["full_name"].casefold() == TARGET.casefold():
        raise RuntimeError(f"PR #{source['number']} unexpectedly has a trusted target-repo head")
    api(f"repos/{TARGET}/issues/{created['number']}", method="PATCH",
        payload={"labels": sorted(set(labels))})
    ensure_comments(created["number"], source["number"], include_reviews=True)
    return created


def build_snapshot(source_items: list[dict[str, Any]],
                   pulls_by_number: dict[int, dict[str, Any]]) -> dict[str, Any]:
    items: list[dict[str, Any]] = []
    for index, item in enumerate(sorted(source_items, key=lambda value: value["number"]), 1):
        number = item["number"]
        print(f"snapshot #{number} ({index}/{len(source_items)})", flush=True)
        record: dict[str, Any] = {
            "issue": item,
            "comments": api_list(f"repos/{SOURCE}/issues/{number}/comments?per_page=100"),
            "events": api_list(f"repos/{SOURCE}/issues/{number}/events?per_page=100"),
        }
        if number in pulls_by_number:
            record.update({
                "pull_request": pulls_by_number[number],
                "reviews": api_list(f"repos/{SOURCE}/pulls/{number}/reviews?per_page=100"),
                "review_comments": api_list(f"repos/{SOURCE}/pulls/{number}/comments?per_page=100"),
                "commits": api_list(f"repos/{SOURCE}/pulls/{number}/commits?per_page=100"),
                "files": api_list(f"repos/{SOURCE}/pulls/{number}/files?per_page=100"),
            })
        items.append(record)
    return {"source": SOURCE, "target": TARGET,
            "captured_from_public_github_api": True, "items": items}


def main() -> int:
    assert_actions_disabled()
    ensure_labels()
    existing = target_items_by_marker()
    mapping: list[dict[str, Any]] = []
    source_items = api_list(f"repos/{SOURCE}/issues?state=all&per_page=100")
    pulls = sorted(api_list(f"repos/{SOURCE}/pulls?state=all&per_page=100"),
                   key=lambda item: item["number"])
    pulls_by_number = {pull["number"]: pull for pull in pulls}
    source_numbers = sorted(item["number"] for item in source_items)
    if source_numbers != list(range(1, max(source_numbers) + 1)):
        raise RuntimeError("source tracker numbering is not contiguous; exact migration is unsafe")
    target_items = api_list(f"repos/{TARGET}/issues?state=all&per_page=100")
    unmarked = [item["number"] for item in target_items if not re.search(
        rf"<!-- upstream-migration:{re.escape(SOURCE)}:(issue|pr):(\d+) -->",
        item.get("body") or "",
    )]
    if unmarked:
        raise RuntimeError(f"target tracker has non-migration items: {unmarked}")

    SNAPSHOT_PATH.write_text(
        json.dumps(build_snapshot(source_items, pulls_by_number),
                   ensure_ascii=False, indent=2) + "\n", encoding="utf-8",
    )
    for source_stub in sorted(source_items, key=lambda item: item["number"]):
        number = source_stub["number"]
        pull = pulls_by_number.get(number)
        kind = "pr" if pull is not None else "issue"
        source = pull or source_stub
        target = existing.get(f"{kind}:{number}")
        if target is None:
            if pull is None:
                target = create_issue(source, archive_pr=False)
            elif pull["state"] == "open":
                target = create_open_pr(pull)
            else:
                target = create_issue(pull, archive_pr=True)
        if target["number"] != number:
            raise RuntimeError(
                f"number preservation failed: source #{number} -> target #{target['number']}"
            )
        ensure_comments(target["number"], number, include_reviews=pull is not None)
        mapping.append({
            "kind": "issue" if pull is None else (
                "pull_request" if pull["state"] == "open" else "pull_request_archive"
            ),
            "source": number, "target": target["number"],
            "target_url": target["html_url"], "source_state": source["state"],
            "source_merged_at": pull.get("merged_at") if pull else None,
        })
        print(f"{kind} #{number} -> #{target['number']}", flush=True)

    MAP_PATH.write_text(json.dumps({
        "source": SOURCE, "target": TARGET,
        "issues": len(source_items) - len(pulls), "pull_requests": len(pulls),
        "mapping": mapping,
    }, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"mapping: {MAP_PATH}")
    print(f"snapshot: {SNAPSHOT_PATH}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"migration failed: {error}", file=sys.stderr)
        raise
