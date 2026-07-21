# Windows detached stdio isolation implementation plan

**Goal:** Ensure asynchronous Windows launch closes the caller's redirected stdout pipe when `v8-runner` exits, independently of the 1C client lifetime.

**Architecture:** Keep `std::process::Command` and existing Job Object integration. Add a Windows-only, idempotent standard-handle isolation step at the shared detached spawn boundary using `windows-sys`.

**Tech Stack:** Rust 2021, `std::process`, `windows-sys`, existing process runner and integration-test harness.

---

## Task 1: Lock the process-mode contract with tests

**Files:**
- Modify: `src/platform/process.rs`

1. Add a cross-platform unit test showing detached and managed-detached modes require standard-handle isolation while captured mode does not.
2. Run the focused test and record RED before implementation.

## Task 2: Isolate inherited Windows standard handles

**Files:**
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `src/platform/process.rs`

1. Add the target-specific `windows-sys` features needed for `GetStdHandle` and `SetHandleInformation`.
2. Implement null/invalid-handle filtering and fail-closed error propagation for valid handles.
3. Invoke isolation for both detached modes before every spawn attempt; keep captured execution unchanged.
4. Run focused process tests and Windows-target compilation when available.

## Task 3: Add the Windows EOF regression

**Files:**
- Modify: `src/platform/process.rs`

1. Add a Windows-only subprocess-of-test helper that launches a sleeping detached process.
2. Assert that the helper's redirected stdout reaches EOF promptly while the detached PID remains alive.
3. Clean up only the returned process tree.

## Task 4: Verify and review

**Files:**
- Review: all branch changes against `upstream/master`

1. Run formatting, diff check, process and launch tests, `cargo check --all-targets`, and available Windows checks.
2. Run independent tester, reviewer, and Rust-expert checklist passes.
3. Fix every finding or record a justified waiver, then push and open an upstream PR closing #31.
