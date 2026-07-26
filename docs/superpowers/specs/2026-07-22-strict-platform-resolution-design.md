# Strict Platform Resolution Design

## Goal

Make a configured 1C platform path an explicit search boundary and make version enforcement
predictable across path and non-path discovery.

## Configuration contract

Maintainer decision: `tools.platform.path` itself means "search only here". It does not fall back to
default roots or `PATH`, regardless of `strict`. `strict` is a boolean with default `false`; it does
not require `tools.platform.path`.

Contract matrix:

| Configuration | Behavior |
| --- | --- |
| `version`, no `path` | Search default roots and `PATH` with version filtering. |
| `path + version`, `strict: false` | Search only `path`; ignore `version`. |
| `path + version`, `strict: true` | Search only `path`; require a matching version. |
| `path`, no `version` | Search only `path`; do not check version. |
| No `path`, no `version` | Normal default-root and `PATH` discovery. |
| `strict: true`, no `path` | No boundary is created; with `version`, behaves like version-only discovery; without `version`, it is a no-op. |

Version requirements retain the existing semantics when they are applied: four components are exact;
two or three components are prefixes and select the highest matching installation.

## Installation consistency

When `strict: true` is combined with `path`, the first successfully resolved platform utility binds
the locator to its canonical installation root. Later resolution of `1cv8`, `1cv8c`, or `ibcmd`
uses only a direct or `bin` sibling below that root. A sibling elsewhere is rejected. For a file
path hint, `strict: false` resolves lexical siblings next to the configured file; `strict: true`
resolves siblings from the canonical installation.

Each location carries a typed resolution source (`explicit`, `default-root`, or `path`), an absolute canonical executable path, inferred version, and canonical installation root.

## JSON scope

`launch` already exposes its selected binary as a public result. It will additionally expose structured platform resolution metadata containing absolute path, version, source, and installation root. Extending every command result would require a separate shared-envelope contract migration and is outside this focused locator fix.

## Verification

TDD covers version-only discovery, path-only no-fallback behavior, lenient path+version ignoring,
strict path+version mismatch and unknown-version errors, versioned-root selection, sibling
consistency, relative path normalization, schema validation, and launch JSON metadata.
