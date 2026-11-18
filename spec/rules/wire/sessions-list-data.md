---
id: CTR.WIRE.SESSIONS-LIST-DATA
version: 1
artifact: docs/schemas/command-data/sessions-list.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/cli_sessions.rs::terminate_all_includes_application_jobs_and_leaves_protected_sessions
---

# `data` команды `sessions list`

Перечень содержит `infobase`, `managed_ras` и `sessions`. Строка сеанса общая с `error.sessions.list`: `id` — UUID, `number` — номер, `computer`, `application`, `application_title`, `started`, `user`, `from_this_machine` и `excluded`. Пустой список — успешный ответ.

`managed_ras` показывает, поднял ли раннер RAS на время команды.

## Пример

```json
{
  "infobase": "origin",
  "managed_ras": false,
  "sessions": [
    {
      "id": "11111111-1111-1111-1111-111111111111",
      "number": 42,
      "computer": "workstation",
      "application": "1CV8C",
      "application_title": "1CV8C",
      "started": "2026-11-16T16:00:00",
      "user": "operator",
      "from_this_machine": false,
      "excluded": false
    }
  ]
}
```
