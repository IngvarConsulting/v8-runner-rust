---
id: CTR.WIRE.SESSIONS-ALLOW-DATA
version: 1
artifact: docs/schemas/command-data/sessions-allow.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/cli_sessions.rs::a_maintenance_window_restores_only_its_own_jobs_change
---

# `data` команды `sessions allow`

Ответ сообщает `infobase`, `managed_ras`, `sessions_denied`, `jobs_denied` и `changed`. Разрешение снимает свою блокировку и восстанавливает состояние заданий, если блокировка его меняла. Чужая блокировка даёт отказ.

`managed_ras` показывает, поднял ли раннер RAS на время команды.

## Пример

```json
{
  "infobase": "origin",
  "managed_ras": false,
  "sessions_denied": false,
  "jobs_denied": true,
  "changed": true
}
```
