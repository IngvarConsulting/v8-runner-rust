---
id: CTR.WIRE.STANDALONE-START-DATA
version: 1
artifact: docs/schemas/command-data/standalone-start.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it
  - src/use_cases/managed_stand/standalone.rs::start_runs_the_server_and_names_its_addresses
---

# `data` команды `standalone start`

`action`: `started`, `already_running` или `planned` в превью. Состояние `state`: `not_running`, `starting`, `started_by_this_copy`, `foreign` — порт записи слушает чужой
процесс, его pid в `occupant_pid`. `connection` — строка прямого шлюза, `gate` — адрес SSH-шлюза, `http` — адрес HTTP.
`stale_record` говорит, что запись была, а процесса из неё уже нет.

## Пример

```json
{
  "ok": true,
  "infobase": "origin",
  "state": "started_by_this_copy",
  "action": "started",
  "pid": 48213,
  "connection": "Srvr=127.0.0.1:24641;Ref=demo",
  "gate": "127.0.0.1:24642",
  "http": "http://127.0.0.1:24643/demo",
  "ready": true,
  "duration_ms": 14200
}
```
