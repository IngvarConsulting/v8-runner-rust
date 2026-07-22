# Test Without Build Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an opt-in CLI test mode that runs against an already prepared infobase without building it.

**Architecture:** Map the CLI flag into a typed transport-neutral build policy. Branch in the test coordinator before artifact creation: validate a file infobase and emit a skipped build step, or execute the existing build-first path.

**Tech Stack:** Rust, clap, serde, existing execution-step and test-result models.

## Global Constraints

- Default test behavior remains build-first.
- MCP remains build-first.
- Skip mode must not invoke build/load/update operations.
- File infobases require an existing `1Cv8.1CD` marker.
- Use Result-based errors and exhaustive enum matches.

---

### Task 1: CLI and typed request policy

**Files:**
- Modify: `src/cli/args.rs`
- Modify: `src/cli/execute.rs`
- Modify: `src/use_cases/request.rs`
- Test: `tests/cli_help.rs`

**Interfaces:**
- Produces: `TestBuildPolicy::{BuildFirst, Skip}` and `TestRequest.build_policy`.

- [ ] Add a failing help test for `test --no-build`.
- [ ] Run the focused help test and confirm it fails because the flag is absent.
- [ ] Add `--no-build`, the enum, and mapping with build-first defaults for non-CLI callers.
- [ ] Run the focused help test and request-mapping tests.

### Task 2: Skip behavior and file-infobase preflight

**Files:**
- Modify: `src/use_cases/run_tests/coordinator.rs`
- Modify: `src/use_cases/run_tests/helpers.rs`
- Modify: `src/domain/test.rs`
- Test: `tests/cli_test.rs`

**Interfaces:**
- Consumes: `TestRequest.build_policy`.
- Produces: skipped `build` step or typed `infobase_unavailable` failure.

- [ ] Add failing YaXUnit tests for skipped build and missing `1Cv8.1CD`.
- [ ] Run them and confirm the missing option/behavior failures.
- [ ] Implement preflight, skipped step, and exhaustive test error mapping.
- [ ] Run the focused YaXUnit tests.
- [ ] Add a failing Vanessa no-build test, then implement only any missing shared behavior.
- [ ] Run the focused Vanessa test.

### Task 3: Documentation and verification

**Files:**
- Modify: `README.md`
- Modify: `docs/CAPABILITIES.md`
- Modify: `SKILL/SKILL.md`
- Modify: `SKILL/references/testing.md`

**Interfaces:**
- Documents the CLI-only prepared-infobase workflow and file/server distinction.

- [ ] Update user and agent guidance.
- [ ] Run formatter, focused suites, check, and diff-check.
- [ ] Run independent Rust and contract reviews; fix or explicitly waive every finding.
- [ ] Commit, push, and create the upstream PR.
