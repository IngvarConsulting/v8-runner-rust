# Upstream migration record

On 2026-09-02, Ingvar Consulting created the maintained fork from
`alkoleft/v8-runner-rust` at baseline
`7ce1b062843d86644fe55741dbe0ee79f7ca767d`.

The migration preserved the unified issue/pull-request number space `#1..#56`:

- 29 upstream issues became issues with the same numbers and states;
- 14 open pull requests remain cross-fork pull requests against current
  `master`, with their exact upstream head and historical base SHA recorded;
- 13 closed or merged pull requests became closed archival issues because the
  GitHub API cannot recreate historical merged pull requests faithfully;
- comments, reviews, inline comments, source URLs, authors, and timestamps were
  copied with explicit attribution. GitHub necessarily attributes the new
  objects to the migration account;
- active mentions and issue-closing keywords were neutralized in rendered
  copies; the unsanitized public API snapshot is retained as a compressed
  archival record.

Exact open-PR head commits are retained in locked
`quarantine/upstream-pr-<number>` branches. Those branches are evidence only and
are not pull-request heads. The live pull requests continue to use external
fork heads so CI retains GitHub's untrusted-fork boundary.

Seven upstream releases were mirrored after release immutability was enabled.
All 56 original assets were verified by size and SHA-256 before upload. A
separate `LICENSE` asset was added to each mirror. The `v0.2.0` and `v0.3.0`
tags predate the repository license file; their release notes and manifest
identify upstream commit `d2427b12acbb50af1a01071c490720e89d2d4011`, which
added only `LICENSE`, instead of claiming the file existed in those tags.

Files:

- `tracker-map.json` — one-to-one tracker mapping and preserved states;
- `release-mirror-manifest.json` — upstream and mirror release identity,
  original asset hashes, and licensing provenance;
- `upstream-tracker-snapshot.json.gz` and `upstream-releases.json.gz` — raw
  public GitHub API evidence;
- `scripts/` — exact operational scripts used for the one-time migration;
- `SHA256SUMS` — checksums of every retained evidence file and script.

The scripts are historical evidence, not a recurring synchronization service.
Development in this fork proceeds independently; upstream changes are imported
only through an explicit reviewed change.
