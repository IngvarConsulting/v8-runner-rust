---
id: CTR.WIRE.CLUSTER-STATUS-DATA
version: 1
artifact: docs/schemas/command-data/cluster-status.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - src/use_cases/managed_stand/cluster.rs::the_remnants_of_a_fallen_agent_are_stopped_by_their_pid
---

# `data` команды `cluster status`

`action` всегда `reported`. Поля те же, что у `cluster start`. Если агент упал, а его `rmngr` и `rphost` живы,
`remnant_pids` называет их; такие процессы не делают закреплённый порт чужим.

## Пример

```json
{
  "ok": true,
  "infobase": "origin",
  "state": "started_by_this_copy",
  "action": "reported",
  "pid": 6312,
  "ras_pid": 16980,
  "version": "8.5.4.2028",
  "connection": "Srvr=127.0.0.1:46761;Ref=demo",
  "agent": "127.0.0.1:46760",
  "ras": "127.0.0.1:25425",
  "worker_range": "46762:46777",
  "registry_dir": ".local/infobases/origin/managed/cluster/registry-46761",
  "log_path": ".local/infobases/origin/managed/cluster/cluster.log",
  "ready": true,
  "registered": true,
  "duration_ms": 420
}
```
