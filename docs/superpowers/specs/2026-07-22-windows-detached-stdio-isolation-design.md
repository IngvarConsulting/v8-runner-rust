# Windows detached stdio isolation

## Problem

On Windows, `std::process::Command` starts children with handle inheritance enabled so that configured standard streams can reach the child. `Stdio::null()` replaces the child's standard streams, but it does not prevent another inheritable copy of the runner's own stdin, stdout, or stderr handle from entering the child process. A detached 1C client can therefore keep a wrapper's stdout pipe open after `v8-runner` exits.

## Boundary

Before spawning either a detached or managed-detached child on Windows, the process runner clears `HANDLE_FLAG_INHERIT` on the handles returned by `GetStdHandle` for stdin, stdout, and stderr. Null and invalid standard handles are ignored. Query or update failures for otherwise valid handles fail the spawn through the existing typed `ProcessError::SpawnFailed` path.

The change is permanent and idempotent. It does not close the handles or affect the runner's own I/O. Avoiding a temporary clear/restore window also avoids a process-wide concurrency race. Rust continues to create the explicit inheritable NUL handles required by the detached child.

Captured execution is unchanged. Its explicitly created pipe handles remain available to the child, while the runner's original standard handles are no longer inherited after the first detached launch.

## Rejected alternatives

- `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` is the ideal Win32 allowlist, but Rust's `spawn_with_attributes` and `inherit_handles` APIs are unstable. A manual `CreateProcessW` implementation would duplicate command-line quoting, environment, working-directory, executable resolution, child-handle ownership, and Job Object integration.
- Temporarily clearing and restoring inheritance introduces a race with concurrent process creation and makes restoration failure difficult to handle after the child has already started.
- Replacing standard streams with NUL alone is the current behavior and does not remove additional inherited handles.

## Verification

A Windows-only regression runs a subprocess of the test binary with stdout redirected to a pipe. That helper launches a sleeping detached child through `ProcessExecutor` and exits. The outer reader must observe EOF while the returned child PID is still alive, then terminate only that child tree. Cross-platform tests assert that both detached modes select the isolation boundary and captured mode does not.

