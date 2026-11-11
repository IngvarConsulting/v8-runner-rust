---
id: CTR.WIRE.STANDALONE-STATUS-DATA
version: 2
artifact: docs/schemas/command-data/standalone-status.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/cli_standalone_managed.rs::status_answers_while_another_command_holds_the_work_path
---

# `data` команды `standalone status`

`action` всегда `reported`. Состояние `state`: `not_running`, `starting`, `started_by_this_copy`, `foreign` — порт записи слушает чужой
процесс, его pid в `occupant_pid`. `connection` — строка прямого шлюза, `gate` — адрес SSH-шлюза, `http` — адрес HTTP.
`stale_record` говорит, что запись была, а процесса из неё уже нет. Адрес сервера живёт только в состоянии стенда:
занятый блок `standalone start` сменяет (`INV.PLATFORM.STANDALONE-PORTS-AND-KEY-ARE-PINNED`), а `v8project*.yaml`
раннер не правит. Берите адрес из этого ответа и не кешируйте.

## Пример

```json
{
  "ok": true,
  "infobase": "origin",
  "state": "started_by_this_copy",
  "action": "reported",
  "pid": 48213,
  "connection": "Srvr=127.0.0.1:24641;Ref=demo",
  "gate": "127.0.0.1:24642",
  "http": "http://127.0.0.1:24643/demo",
  "ready": true,
  "duration_ms": 14200
}
```
