#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
: "${V8TR_BIN:?Set V8TR_BIN to the built v8-runner binary}"
scratch="$(mktemp -d)"
trap 'rm -rf -- "$scratch"' EXIT

export SMOKE_TEST_REAL_BIN="$V8TR_BIN"
export V8TR_BIN="$scratch/runner"
export V8TR_DESIGNER_SMOKE_PROFILE=mandatory
export V8TR_DESIGNER_TEST_MODE=none
export V8TR_DESIGNER_ALLOW_MISSING_CONFIG=0
export V8TR_DESIGNER_REAL_CONFIG=
export V8TR_IBCMD_REAL_CONFIG=
export V8TR_DESIGNER_WORK_CONFIG="$scratch/designer.yaml"
export V8TR_IBCMD_WORK_CONFIG="$scratch/ibcmd.yaml"

# Run the fixture through its normal entry points. Intercept only the CLI calls
# that would require 1C, and stop after the harness has consumed the first push.
cat > "$V8TR_BIN" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
[[ "$1" == --config ]]
config="$2"
shift 2
case "$*" in
    'infobase create')
        if [[ "$SMOKE_TEST_MODE" == config ]]; then
            "$SMOKE_TEST_REAL_BIN" --config "$config" platforms
            for operation in infobase.create push pull make; do
                grep -Fx "  $operation: $SMOKE_TEST_PROVIDER" "$config"
            done
            exit 71
        fi
        mkdir -p "$V8TR_LIVE_CLI_OUTPUT_ROOT/infobase"
        touch "$V8TR_LIVE_CLI_OUTPUT_ROOT/infobase/1Cv8.1CD"
        ;;
    '--json-message push --full')
        printf '%s\n' '{"ok":true,"command":"push","data":{"steps":[{"source_set":"configuration","ok":true},{"source_set":"Расширение1","ok":true}]}}'
        exit "${SMOKE_TEST_PUSH_EXIT:-0}"
        ;;
    '--json-message push')
        exit 72
        ;;
    *)
        printf 'Unexpected CLI call: %s\n' "$*" >&2
        exit 1
        ;;
esac
SH
chmod +x "$V8TR_BIN"

expect_exit() {
    local expected="$1"
    local actual=0
    shift
    "$@" >"$scratch/run.log" 2>&1 || actual=$?
    if [[ "$actual" != "$expected" ]]; then
        printf 'Expected exit %s, got %s from %s\n' "$expected" "$actual" "$*" >&2
        cat "$scratch/run.log" >&2
        return 1
    fi
}

result=0
for backend in designer ibcmd; do
    wrapper="$SCRIPT_DIR/live-cli-$backend.sh"
    export V8TR_LIVE_CLI_OUTPUT_ROOT="$scratch/$backend"
    SMOKE_TEST_MODE=config SMOKE_TEST_PROVIDER="$backend" expect_exit 71 bash "$wrapper" || result=1
    # Start with an unused root, then prove the harness clears and recreates it.
    export V8TR_LIVE_CLI_OUTPUT_ROOT="$scratch/$backend-receipts"
    for run in fresh repeat; do
        SMOKE_TEST_MODE=receipts expect_exit 72 bash "$wrapper" || result=1
        if [[ ! -s "$V8TR_LIVE_CLI_OUTPUT_ROOT/json/build.json" || -e "$V8TR_LIVE_CLI_OUTPUT_ROOT/stale" ]]; then
            printf 'Expected a fresh build receipt for %s (%s run)\n' "$backend" "$run" >&2
            result=1
        fi
        touch "$V8TR_LIVE_CLI_OUTPUT_ROOT/stale"
    done
    SMOKE_TEST_MODE=receipts SMOKE_TEST_PUSH_EXIT=73 expect_exit 73 bash "$wrapper" || result=1
done

(
    unset GITHUB_ENV V8TR_INFOBASE_PATH
    export V8TR_CI_RUNTIME_ROOT="$scratch/ci-runtime"
    export V8TR_DESIGNER_REAL_CONFIG="$V8TR_CI_RUNTIME_ROOT/live.yaml"
    export V8TR_LIVE_CLI_OUTPUT_ROOT="$scratch/ci-output"
    export V8TR_PLATFORM_PATH="$scratch/platform"
    export V8TR_LIVE_CLI_BACKEND=DESIGNER
    bash "$SCRIPT_DIR/ci-designer-config.sh" &&
        SMOKE_TEST_MODE=config SMOKE_TEST_PROVIDER=designer expect_exit 71 bash "$SCRIPT_DIR/live-cli-fixture.sh"
) || result=1

exit "$result"
