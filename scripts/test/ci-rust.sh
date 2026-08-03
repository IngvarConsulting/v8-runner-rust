#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CI_SCOPE="${V8_RUNNER_CI_SCOPE:-contract}"
TARGET_OS_LABEL="${V8TR_CI_TARGET_OS:-$(uname -s)}"

cd "$ROOT_DIR"

case "$CI_SCOPE" in
  contract)
    case "$TARGET_OS_LABEL" in
      Windows|MINGW*|MSYS*|CYGWIN*)
        echo "Windows contract scope runs compile/check smoke plus the detached stdio regression; full cargo test remains Linux-owned until the Windows test suite is hardened."
        cargo check --locked --all-targets
        windows_stdio_tests=(
          "platform::process::tests::detached_child_does_not_hold_redirected_stdout_open"
          "platform::process::tests::managed_detached_child_does_not_hold_redirected_stdout_open"
        )
        listed_tests="$(cargo test --locked -- --list)"
        for test_name in "${windows_stdio_tests[@]}"; do
          if ! grep -Fxq "$test_name: test" <<<"$listed_tests"; then
            echo "Windows detached stdio regression is missing: $test_name" >&2
            exit 2
          fi
          cargo test --locked "$test_name" -- --exact --nocapture
        done
        ;;
      *)
        cargo test --locked
        ;;
    esac
    ;;
  full)
    cargo test --locked
    ;;
  runtime-locks)
    cargo test --locked workspace_lock
    cargo test --locked advisory_lock
    cargo test --locked execute_command_reports_workspace_lock_conflict
    cargo test --locked default_port_reports_workspace_lock_conflict_before_use_case_dispatch
    ;;
  happy-path)
    bash "$ROOT_DIR/scripts/test/ci-happy-path.sh"
    ;;
  *)
    echo "Unsupported V8_RUNNER_CI_SCOPE: $CI_SCOPE" >&2
    echo "Expected one of: contract, full, runtime-locks, happy-path" >&2
    exit 2
    ;;
esac
