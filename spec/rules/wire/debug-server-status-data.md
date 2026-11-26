---
id: CTR.WIRE.DEBUG-SERVER-STATUS-DATA
version: 2
artifact: docs/schemas/command-data/debug-server-status.schema.json
check:
  - src/use_cases/managed_stand/cluster.rs::cluster_debug_status_requires_the_same_registration_and_direct_listener
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/cli_debug_server.rs::status_reports_an_unknown_session_without_waiting_for_the_work_path
  - src/platform/service/macos_listener.rs::macos_proves_only_an_exact_loopback_listener_held_by_the_pid
  - src/platform/service/macos_listener.rs::macos_finds_the_accepted_socket_by_both_endpoints
  - src/platform/service/linux_listener.rs::linux_debug_listener_requires_an_unambiguous_loopback_socket_held_by_the_pid
  - src/platform/dbgs/owned_rdbg.rs::owned_rdbg_requires_the_same_accepted_connection_after_the_reply
  - src/platform/dbgs/owned_rdbg.rs::owned_rdbg_does_not_send_a_request_to_an_unattributed_connection
  - tests/cli_debug_server.rs::start_runs_the_debug_server_beside_the_client_and_stop_ends_it
  - src/use_cases/managed_stand/debug_server.rs::status_confirms_only_the_recorded_listener_without_mutating_the_session
---

# `data` команды `debug-server status`

`session` — идентификатор сеанса из вызова в нижнем регистре. `status` — результат
проверки сеанса в текущей рабочей копии для файловой ИБ или ИБ управляемого кластера.

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

Для управляемого кластера `ready` дополнительно требует совпадения зарегистрированного
`Ref`, UUID кластера и регистрации ИБ, идентичности `ragent` и закреплённого порта.
Слушатель должен быть дочерним `dbgs` этого `ragent`. После ответа `rdbg` раннер повторяет
сверку регистрации и процесса. `pid` называет слушателя `dbgs`, а `infobase_alias` — `Ref`.
Метки, выданные до привязки к регистрации, и наблюдения без подтверждённой принадлежности
слушателя возвращают `unverified` без адреса.

Проверка ничего не меняет в записи сеанса или кластере и не запускает или завершает
службы. Для кластера запускается `rac` только на чтение регистрации.
На Linux и macOS раннер убеждается, что записанному процессу принадлежит и слушатель, и
принятый сокет конкретного запроса `rdbg`. Принятый сокет проверяется до запроса
и после ответа по локальному и удалённому адресам, портам и идентификатору TCP-записи.
На Linux раннер сверяет точный IPv4 loopback-слушатель и inode принятого сокета с дескрипторами
записанного процесса. Недоступная таблица TCPv6 или IPv6 wildcard на этом порту оставляют наблюдение
неоднозначным. При неоднозначном слушателе или отсутствии доказательства принадлежности действует `unverified`.

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

```json
{
  "session": "5f0c2a9d4e3b4c1f8a7d6e5c4b3a2f10",
  "status": "ready",
  "pid": 18752,
  "port": 46497,
  "url": "http://127.0.0.1:46497",
  "infobase_alias": "registered-demo",
  "duration_ms": 180
}
```
