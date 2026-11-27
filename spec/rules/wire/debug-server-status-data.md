---
id: CTR.WIRE.DEBUG-SERVER-STATUS-DATA
version: 3
artifact: docs/schemas/command-data/debug-server-status.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - src/use_cases/managed_stand/debug_server.rs::status_confirms_only_the_recorded_listener_without_mutating_the_session
  - src/use_cases/managed_stand/debug_server.rs::status_refuses_a_lease_that_begins_stopping_during_rdbg
  - tests/cli_debug_server.rs::status_reports_an_unknown_session_without_waiting_for_the_work_path
  - src/platform/service/macos_listener.rs::macos_proves_only_an_exact_loopback_listener_held_by_the_pid
  - src/platform/service/macos_listener.rs::macos_finds_the_accepted_socket_by_both_endpoints
  - src/platform/service/linux_listener.rs::linux_debug_listener_requires_an_unambiguous_loopback_socket_held_by_the_pid
  - src/platform/dbgs/owned_rdbg.rs::owned_rdbg_requires_the_same_accepted_connection_after_the_reply
  - src/platform/dbgs/owned_rdbg.rs::owned_rdbg_does_not_send_a_request_to_an_unattributed_connection
  - tests/cli_debug_server.rs::start_runs_the_debug_server_beside_the_client_and_stop_ends_it
---

# `data` команды `debug-server status`

`session` — переданная аренда из `release.session` команды `debug-server start`,
в нижнем регистре. Наблюдение относится к собственному `dbgs` текущей рабочей
копии. Заимствованный сервер возвращает `release: null`.

- `ready` подтверждает записанные машину, PID и время запуска процесса, его
  loopback-слушатель и ответ RDBG. Результат содержит `pid`, `port`, `url` и
  `infobase_alias` на момент наблюдения.
- `unknown` — у рабочей копии нет этой аренды.
- `stale` — процесс завершился или его идентичность сменилась.
- `unverified` — наблюдение недостаточно для подтверждения готовности.

Раннер проверяет принадлежность сокета и идентичность процесса вокруг
запроса к RDBG. На Linux и macOS проверяется принятый сокет конкретного
соединения. Одновременно мог измениться и сам токен аренды. Проверка только
читает запись и опрашивает сервер.

## Пример

```json
{
  "session": "5f0c2a9d4e3b8c1f8a7d6e5c4b3a2f10",
  "status": "ready",
  "pid": 1280,
  "port": 13872,
  "url": "http://127.0.0.1:13872",
  "infobase_alias": "DefAlias",
  "duration_ms": 35
}
```
