---
id: CTR.WIRE.CLUSTER-STOP-DATA
version: 1
artifact: docs/schemas/command-data/cluster-stop.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it
  - src/use_cases/managed_stand/cluster.rs::stop_takes_ras_then_the_agent_then_the_step_postgres
---

# `data` команды `cluster stop`

`action`: `stopped`, `nothing_to_stop` или `planned` в превью. `stopped_pid` — агент, `stopped_ras_pid` —
`ras`, `stopped_remnant_pids` — остатки упавшего агента, снятые по их pid, `stopped_dbms_pid` — PostgreSQL,
поднятый шагом кластера.

## Пример

```json
{
  "ok": true,
  "infobase": "origin",
  "state": "not_running",
  "action": "stopped",
  "ready": false,
  "registered": false,
  "stopped_pid": 6312,
  "stopped_ras_pid": 16980,
  "stopped_dbms_pid": 15604,
  "duration_ms": 3300
}
```
