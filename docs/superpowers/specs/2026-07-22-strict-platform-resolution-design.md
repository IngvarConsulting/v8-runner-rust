# Strict Platform Resolution Design

## Goal

Make an explicitly pinned 1C platform installation fail closed when requested, while preserving the legacy discovery fallback by default.

## Configuration contract

`tools.platform.strict` is a boolean with default `false`. `strict: true` requires `tools.platform.path`. The path is normalized relative to the primary config directory.

When strict mode is disabled, explicit path, default installation roots, and PATH keep the current fallback order. When strict mode is enabled, only the explicit path boundary is searched. A missing requested utility, an unknown version when `tools.platform.version` is configured, or a version mismatch is a typed locator error and never falls back.

Version requirements retain the existing semantics: four components are exact; two or three components are prefixes and select the highest matching installation below an explicit version root.

## Installation consistency

In strict mode, the first successfully resolved platform utility binds the locator to its canonical installation root. Later resolution of `1cv8`, `1cv8c`, or `ibcmd` uses only a direct or `bin` sibling below that root. A sibling elsewhere in default roots or PATH is rejected.

Each location carries a typed resolution source (`explicit`, `default-root`, or `path`), an absolute canonical executable path, inferred version, and canonical installation root.

## JSON scope

`launch` already exposes its selected binary as a public result. It will additionally expose structured platform resolution metadata containing absolute path, version, source, and installation root. Extending every command result would require a separate shared-envelope contract migration and is outside this focused locator fix.

## Verification

TDD covers strict missing paths, exact and prefix mismatch, unknown pinned version, versioned-root selection, sibling consistency, legacy fallback, relative path normalization, schema validation, and launch JSON metadata. Default behavior and existing locator tests remain unchanged.
