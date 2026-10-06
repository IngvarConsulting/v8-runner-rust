---
id: INV.CLI.THE-BASE-LOCK-FOLLOWS-THE-WORKPATH-LOCK
check:
  - tests/cli_infobase_lock.rs::a_busy_work_path_is_refused_before_the_base_lock
  - src/use_cases/transport.rs::dispatch_with_workspace_lock_stops_before_run_when_workspace_is_busy
  - src/use_cases/transport.rs::a_held_base_stops_the_dispatch_after_the_workspace_lock
---

# Замок базы берётся после замка `workPath`

Команда берёт замок файловой базы после замка своего `workPath`.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).
