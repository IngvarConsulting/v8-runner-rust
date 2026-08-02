# Strict Platform Resolution Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add fail-closed platform pinning with coherent installation resolution and observable launch metadata.

**Architecture:** Add a typed resolution policy at the config-to-locator boundary. Candidates carry their source; any configured platform path searches only that explicit boundary, while strict path resolution also enforces version and pins one canonical installation root for all platform utilities.

**Tech Stack:** Rust, serde, schemars, clap integration tests, existing locator and launch contracts.

## Task 1: Configuration and path normalization

- [ ] Add failing model/schema/loader tests for strict, relative platform paths, and strict without path.
- [ ] Add `PlatformToolConfig.strict`, schema fields, strict-without-path acceptance, and path normalization.
- [ ] Regenerate both checked-in schemas and run focused config tests.

## Task 2: Typed strict locator

- [ ] Add failing locator tests for the path/version/strict contract matrix, exact/prefix mismatch, unknown version, and sibling consistency.
- [ ] Add typed policy/source/errors and source-aware candidates.
- [ ] Make configured path resolution explicit-only, bind strict path resolution to one canonical installation root, and capture PATH roots once.
- [ ] Run the complete locator and utilities suites.

## Task 3: JSON and documentation

- [ ] Add a failing launch JSON test for path/version/source/root metadata.
- [ ] Extend LaunchResult and mapping without removing the existing binary field.
- [ ] Update configuration, capabilities, and repo-local skill guidance.
- [ ] Run formatting, focused integration tests, all-target check, and clippy.
- [ ] Run independent tester, reviewer, and Rust expert passes; resolve or waive every finding.
- [ ] Commit, push, and create the upstream PR.
