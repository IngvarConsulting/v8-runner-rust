# Test Without Build Design

## Goal

Allow CLI users to run YaXUnit or Vanessa Automation against a prepared infobase without invoking the build pipeline.

## Contract

`v8-runner test --no-build yaxunit all` and `v8-runner test --no-build va` select a typed `TestBuildPolicy::Skip`. The default remains `TestBuildPolicy::BuildFirst`. MCP requests keep the default and do not expose the new CLI-only option.

The result keeps a `build` execution step. In skip mode that step has status `skipped` and a stable message stating that the caller explicitly skipped the prerequisite.

## Infobase preflight

For file connections, skip mode requires the configured infobase directory to contain `1Cv8.1CD`. Failure is returned before test artifacts or a platform process are created, using typed test error code `infobase_unavailable`.

Prepared-test config loading deliberately skips source-set and build-tool filesystem validation, while retaining base/work path, connection, platform, timeout, and test configuration validation. This lets immutable CI clones run after their configuration sources have been removed.

Accepted portability waiver: the current public server connection contract does not carry cluster-administration credentials or a portable non-1C management adapter, so existence of a named server infobase cannot be proven before starting 1C without introducing a false-positive TCP check or a new external dependency. Server availability therefore remains established by the test-engine connection and its typed process errors; file infobases receive the strict preflight required by this change.

Server connections cannot be proven available without contacting the server through a platform process. Skip mode therefore validates their configuration using the existing loader and lets the selected test engine establish connectivity; it does not introduce a hidden probe command.

## Components

- CLI maps `--no-build` to the typed build policy.
- The transport-neutral request owns the policy.
- The test coordinator performs file-infobase preflight or the existing build prerequisite.
- Existing step serialization reports the explicit skip.
- CLI integration tests cover YaXUnit, Vanessa, and a missing file infobase.
- README, capabilities, and repo-local skill guidance describe the workflow.

## Error handling and compatibility

Default behavior is unchanged. Skip mode never invokes build, load, or update-database operations. Missing file state returns the existing runtime CLI error class plus the new typed test error. Credentials and launch options continue through existing code paths.

## Testing

Tests first demonstrate that the option is absent. After implementation they assert no build script invocation, a skipped build step in JSON, successful YaXUnit and Vanessa execution, and failure before platform launch when `1Cv8.1CD` is missing.
