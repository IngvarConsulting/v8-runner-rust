# Task 3 report — Aggregate native reports and classify terminal results

## TDD evidence

### RED

1. `cargo test use_cases::run_tests parsers::junit -- --nocapture` was rejected by Cargo because it accepts one filter argument only.
2. `cargo test use_cases::run_tests -- --nocapture` then failed as expected with unresolved `classify_test_completion`, `discover_junit_reports`, `parse_junit_reports`, and `validate_allure_results` imports.
3. After the first independent Rust review, `cargo test use_cases::run_tests::tests::vanessa_junit_parse_failure_inventories_materialized_runner_log -- --nocapture` failed as expected: an empty existing `junit/` directory did not put its path in `JunitNotProduced` details.

### GREEN

- `cargo fmt --check` — passed.
- `cargo test use_cases::run_tests -- --nocapture` — passed (23 tests).
- `cargo test parsers::junit -- --nocapture` — passed (6 tests).
- `cargo test --test cli_test -- --nocapture` — passed (20 tests).
- `git diff --check` — passed.

The suite continues to emit the pre-existing warning for unused `error` in `src/use_cases/tool_extension.rs:126`; it is unrelated to this task.

## Classification evidence

| Valid summary | Native exit | Result |
| --- | ---: | --- |
| `failed > 0` | 1 | `test_failures`, `failed` |
| `errors > 0` | 2 | `test_failures`, `failed` |
| green | 1 | `enterprise_exited_non_zero`, `failed` |
| green | 0 | success, `succeeded` |

`classifies_native_reports_before_process_exit_status` tests the table literally. When a report proves failures and the process exits nonzero, the exit code is retained in diagnostics.

## Delivered behavior

- Recursive sorted JUnit discovery uses `symlink_metadata` and ignores symlinked files/directories.
- Every discovered XML is parsed; summaries saturate, suites/errors aggregate, and all malformed/empty reports return typed errors with their paths.
- Empty or missing Allure results are typed invalid output; nested regular files are accepted and symlinked files do not count.
- Vanessa runner logs are materialized before report collection; runner-log extracted errors append to report errors.
- JUnit/Allure validation precedes runner-log parsing and report-first classification. The external-output `expect` was removed.
- Existing artifact inventory is attached to all terminal outcomes, including successful runs; successful run directories are retained.
- CLI fake runners now generate both native outputs, success assertions require existing artifact paths, and snapshots cover the added validation step.

## Review findings and fixes

- Explorer and skeptic passes identified unordered single-report parsing, absent Allure validation, inverted exit precedence, missing success artifacts, and the external-output panic; all were fixed.
- Independent Rust review found that an empty existing JUnit directory lacked a path diagnostic. A RED regression was added, the helper now adds `JUnit report directory: <path>`, and GREEN was verified.
- Independent tester initially found `cli_test` fixture/snapshot regressions caused by the new contract. Vanessa fake output now creates a native Allure file, success validates existing artifacts, snapshots were updated, and `cli_test` is GREEN.

## Commit and concerns

Commit: `feat(test): classify results from native reports` (SHA reported in handoff).

No accepted waivers. The only remaining warning is the unrelated baseline warning noted above.
