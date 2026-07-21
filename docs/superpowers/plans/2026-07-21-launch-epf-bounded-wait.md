# External EPF bounded wait implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an opt-in bounded, structured wait result for direct thin-client external EPF launches.

**Architecture:** Keep the existing detached path unchanged. Route the opt-in request through typed validation and a managed child process so timeout cleanup and artifact reporting are explicit.

**Tech Stack:** Rust, clap, serde, existing `ProcessRunner` managed-spawn API, CLI integration tests.

## Global Constraints

- Default launch is asynchronous.
- Wait mode accepts only `launch thin` and an explicit `.epf` execute path.
- Wait mode rejects normalized raw aliases `/C`, `/Execute`, and `/Out`.
- Do not include credentials in diagnostics.

### Task 1: Public request/result contract

**Files:** `src/cli/args.rs`, `src/use_cases/request.rs`, `src/domain/launch.rs`, `src/cli/execute.rs`, tests in `tests/cli_launch.rs`.

- [ ] Write a failing JSON CLI test expecting a bounded outcome for `--wait-for-exit`.
- [ ] Run that test and confirm the option is rejected before implementation.
- [ ] Add typed wait request and serializable outcome fields, then map CLI options.
- [ ] Re-run the focused test and confirm it reaches the unimplemented execution path.

### Task 2: Preflight and managed execution

**Files:** `src/use_cases/launch_app.rs`, `src/platform/process.rs`, tests in `tests/cli_launch.rs` and module tests.

- [ ] Write failing tests for direct EPF success, timeout cleanup, and raw-key rejection.
- [ ] Run each focused test and confirm the current detached implementation fails it.
- [ ] Add preflight validation and managed wait/cleanup using the existing process policy.
- [ ] Re-run focused tests and confirm they pass.

### Task 3: Regression and review

**Files:** changed files and `SKILL/SKILL.md` only if external workflow guidance changes.

- [ ] Run formatter, focused CLI suite, unit tests, and relevant clippy check.
- [ ] Obtain independent tester, reviewer, and Rust-expert reviews; fix or record every finding.
- [ ] Rebase onto current upstream master, run the focused suite again, commit and open a PR.
