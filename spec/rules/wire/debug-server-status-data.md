---
id: CTR.WIRE.DEBUG-SERVER-STATUS-DATA
version: 1
artifact: docs/schemas/command-data/debug-server-status.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/cli_debug_server.rs::status_reports_an_unknown_session_without_waiting_for_the_work_path
  - src/use_cases/managed_stand/debug_server.rs::status_confirms_only_the_recorded_listener_without_mutating_the_session
---

# `data` команды `debug-server status`

`session` — идентификатор сеанса из вызова в нижнем регистре. `status` — результат
проверки сеанса в текущей рабочей копии для файловой ИБ.

- `ready` подтверждает записанные машину, PID и время запуска процесса, принадлежность
  ему loopback-порта и ответ `rdbg`. Идентичность процесса и владельца сокета проверяют
  повторно после ответа. Только в этом состоянии возвращаются `pid`, `url`, `port` и
  `infobase_alias`. Состояние наблюдаемое на момент проверки.
- `unknown` — у копии нет записи сеанса.
- `stale` — записанный процесс завершился (`process_gone`) или его идентичность сменилась
  (`process_changed`).
- `unverified` — готовность подтвердить нельзя. `reason` отличает запуск сеанса (`starting`),
  недоступную запись (`record_unavailable`), смену записи во время проверки (`record_changed`),
  владельца вне области проверки (`unsupported_owner`), отсутствие надёжной идентичности
  процесса (`process_unverified`), отсутствие доказательства принадлежности порта
  (`listener_unverified`) или ответа протокола (`rdbg_unavailable`).

Проверка читает запись, ничего в ней не меняет, процессы не запускает и не завершает.
На ОС без доказательства принадлежности порта действует `unverified`.

## Пример

```json
{
  "session": "5f0c2a9d4e3b4c1f8a7d6e5c4b3a2f10",
  "status": "ready",
  "pid": 1280,
  "port": 13872,
  "url": "http://127.0.0.1:13872",
  "infobase_alias": "DefAlias",
  "duration_ms": 35
}
```

```json
{
  "session": "5f0c2a9d4e3b4c1f8a7d6e5c4b3a2f10",
  "status": "unverified",
  "reason": "rdbg_unavailable",
  "duration_ms": 2004
}
```
