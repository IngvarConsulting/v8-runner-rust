---
id: CTR.WIRE.CLUSTER-RESTART-DATA
version: 1
artifact: docs/schemas/command-data/cluster-restart.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it
  - src/use_cases/managed_stand/cluster.rs::restart_leaves_the_postgres_alone
---

# `data` команды `cluster restart`

Поля `cluster start`, а также `stopped_pid`, `stopped_ras_pid` и `stopped_remnant_pids` остановленного кластера.
PostgreSQL перезапуск не трогает.

## Пример

```json
{
  "ok": true,
  "infobase": "origin",
  "state": "started_by_this_copy",
  "action": "started",
  "pid": 6312,
  "ras_pid": 16980,
  "version": "8.5.4.2028",
  "connection": "Srvr=127.0.0.1:46761;Ref=demo",
  "agent": "127.0.0.1:46760",
  "ras": "127.0.0.1:25425",
  "worker_range": "46762:46777",
  "registry_dir": "build/infobases/origin/managed/cluster/registry-46761",
  "log_path": "build/infobases/origin/managed/cluster/cluster.log",
  "ready": true,
  "registered": false,
  "duration_ms": 12400
}
```
