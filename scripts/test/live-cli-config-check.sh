#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
: "${V8TR_BIN:?Set V8TR_BIN to the built v8-runner binary}"
scratch="$(mktemp -d)"
trap 'rm -rf -- "$scratch"' EXIT

result=0
echo "==> Validate Designer smoke config"
V8TR_DESIGNER_REAL_CONFIG= \
V8TR_DESIGNER_WORK_CONFIG="$scratch/designer.yaml" \
V8TR_LIVE_CLI_OUTPUT_ROOT="$scratch/designer" \
    bash "$SCRIPT_DIR/live-cli-designer.sh" --validate-config || result=1

echo "==> Validate IBCMD smoke config"
V8TR_IBCMD_REAL_CONFIG= \
V8TR_IBCMD_WORK_CONFIG="$scratch/ibcmd.yaml" \
V8TR_LIVE_CLI_OUTPUT_ROOT="$scratch/ibcmd" \
    bash "$SCRIPT_DIR/live-cli-ibcmd.sh" --validate-config || result=1

echo "==> Validate trusted CI smoke config"
(
    unset GITHUB_ENV V8TR_INFOBASE_PATH
    export V8TR_CI_RUNTIME_ROOT="$scratch/ci-runtime"
    export V8TR_LIVE_CLI_OUTPUT_ROOT="$scratch/ci-output"
    export V8TR_DESIGNER_REAL_CONFIG="$V8TR_CI_RUNTIME_ROOT/live.yaml"
    export V8TR_PLATFORM_PATH="$scratch/platform"
    bash "$SCRIPT_DIR/ci-designer-config.sh" &&
        V8TR_LIVE_CLI_BACKEND=DESIGNER bash "$SCRIPT_DIR/live-cli-fixture.sh" --validate-config
) || result=1

exit "$result"
