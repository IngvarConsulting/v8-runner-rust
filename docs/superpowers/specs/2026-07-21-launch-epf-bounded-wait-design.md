# Opt-in bounded wait for external EPF launch

## Goal

Make a direct external EPF launch observable without changing the default asynchronous `launch` behaviour.

## Design

`launch thin --wait-for-exit --wait-timeout-ms N` is represented by a typed wait policy. It is valid only with an explicit `.epf` `--execute` value. Before spawning, the use case rejects other launch targets and raw aliases of `/C`, `/Execute`, and `/Out` from both CLI and configured additional keys.

The wait branch keeps a managed child, captures stderr at a caller-declared path, waits within the smaller of the request timeout and command deadline, and uses the existing graceful-then-kill process-group cleanup on timeout or cancellation. Its result serializes the PID, normalized execute proof, exit code or timeout, and declared output/stderr artifacts. Default launch retains its detached `spawn` contract.

## Verification

CLI tests cover a successful direct thin EPF wait, timeout cleanup, invalid targets and raw aliases. Unit tests cover key normalization and connection-string credential redaction. Existing async launch tests must remain unchanged.
