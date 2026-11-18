---
id: CTR.WIRE.SESSIONS-DENY-DATA
version: 1
artifact: docs/schemas/command-data/sessions-deny.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/cli_sessions.rs::a_maintenance_window_restores_only_its_own_jobs_change
---

# `data` команды `sessions deny`

Ответ сообщает `infobase`, `managed_ras`, `sessions_denied`, `jobs_denied` и `changed`. Код разрешения скрыт. Повторный вызов поверх своей блокировки возвращает её состояние.

`managed_ras` показывает, поднял ли раннер RAS на время команды.

## Пример

```json
{
  "infobase": "origin",
  "managed_ras": false,
  "sessions_denied": true,
  "jobs_denied": true,
  "changed": true
}
```
