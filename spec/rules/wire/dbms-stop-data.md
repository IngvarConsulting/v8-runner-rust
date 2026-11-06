---
id: CTR.WIRE.DBMS-STOP-DATA
version: 1
artifact: docs/schemas/command-data/dbms-stop.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it
  - src/use_cases/managed_stand/mod.rs::stop_that_failed_keeps_the_record_and_a_repeat_goes_on
---

# `data` команды `dbms stop`

`action`: `stopped`, `nothing_to_stop` или `planned` в превью; `stopped_pid` называет остановленный процесс. Состояние `state`: `not_running`, `starting`, `started_by_this_copy`, `foreign` — порт записи слушает чужой
процесс, его pid в `occupant_pid`. Действие `action` называет, что сделала команда. Значения пароля в форме нет:
`password_file` — путь к файлу. `stale_record` говорит, что запись была, а процесса из неё уже нет.

## Пример

```json
{
  "ok": true,
  "provider_dispatched": true,
  "infobase": "origin",
  "state": "not_running",
  "action": "stopped",
  "ready": false,
  "stopped_pid": 40112,
  "duration_ms": 610
}
```
