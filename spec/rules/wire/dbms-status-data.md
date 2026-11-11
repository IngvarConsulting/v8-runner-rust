---
id: CTR.WIRE.DBMS-STATUS-DATA
version: 1
artifact: docs/schemas/command-data/dbms-status.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/cli_dbms.rs::status_answers_while_another_command_holds_the_work_path
---

# `data` команды `dbms status`

`action` всегда `reported`. Состояние `state`: `not_running`, `starting`, `started_by_this_copy`, `foreign` — порт записи слушает чужой
процесс, его pid в `occupant_pid`. Действие `action` называет, что сделала команда. Значения пароля в форме нет:
`password_file` — путь к файлу. `stale_record` говорит, что запись была, а процесса из неё уже нет.

## Пример

```json
{
  "ok": true,
  "infobase": "origin",
  "state": "started_by_this_copy",
  "action": "reported",
  "pid": 40112,
  "version": "17.11",
  "address": "127.0.0.1 port=24432",
  "user": "postgres",
  "password_file": ".local/infobases/origin/managed/postgres/password",
  "explicit": true,
  "ready": true,
  "duration_ms": 1240
}
```
