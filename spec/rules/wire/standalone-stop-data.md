---
id: CTR.WIRE.STANDALONE-STOP-DATA
version: 1
artifact: docs/schemas/command-data/standalone-stop.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it
  - src/use_cases/managed_stand/standalone.rs::stop_is_soft_first_and_confirms_the_end_of_the_process
  - src/use_cases/managed_stand/standalone.rs::after_a_reboot_the_server_starts_again_with_its_postgres_as_a_step
  - src/cli/execute.rs::standalone_stop_names_the_postgres_it_stopped_with_the_server
---

# `data` команды `standalone stop`

`action`: `stopped`, `nothing_to_stop` или `planned` в превью. Состояние `state`: `not_running`, `starting`, `started_by_this_copy`, `foreign` — порт записи слушает чужой
процесс, его pid в `occupant_pid`. `connection` — строка прямого шлюза, `gate` — адрес SSH-шлюза, `http` — адрес HTTP.
`stale_record` говорит, что запись была, а процесса из неё уже нет.
`stopped_pid` — остановленный `ibsrv`, `stopped_dbms_pid` — PostgreSQL, поднятый шагом сервера и остановленный
вместе с ним; текстовый вывод называет его строкой `PostgreSQL stopped`.

## Пример

```json
{
  "ok": true,
  "infobase": "origin",
  "state": "not_running",
  "action": "stopped",
  "ready": false,
  "stopped_pid": 48213,
  "stopped_dbms_pid": 15604,
  "duration_ms": 2100
}
```
