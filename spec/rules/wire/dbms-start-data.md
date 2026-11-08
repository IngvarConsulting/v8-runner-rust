---
id: CTR.WIRE.DBMS-START-DATA
version: 1
artifact: docs/schemas/command-data/dbms-start.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it
  - src/use_cases/managed_stand/mod.rs::the_first_start_creates_the_data_directory_and_the_second_reuses_it
---

# `data` команды `dbms start`

`action`: `started`, `already_running` или `planned` в превью. Состояние `state`: `not_running`, `starting`, `started_by_this_copy`, `foreign` — порт записи слушает чужой
процесс, его pid в `occupant_pid`. Действие `action` называет, что сделала команда. Значения пароля в форме нет:
`password_file` — путь к файлу. `stale_record` говорит, что запись была, а процесса из неё уже нет.

## Пример

```json
{
  "ok": true,
  "infobase": "origin",
  "state": "started_by_this_copy",
  "action": "started",
  "pid": 40112,
  "version": "17.11",
  "address": "127.0.0.1 port=24432",
  "user": "postgres",
  "password_file": "build/infobases/origin/managed/postgres/password",
  "explicit": true,
  "ready": true,
  "duration_ms": 1240
}
```
