---
id: INV.USE-CASES.THE-STANDALONE-DEBUG-SERVER-STAYS-ON-UNTIL-NO-DEBUG
check:
  - src/use_cases/managed_stand/standalone.rs::a_debug_start_raises_the_server_with_its_debug_server
  - src/use_cases/managed_stand/standalone.rs::the_debug_server_of_the_standalone_server_stays_on_until_no_debug
  - src/use_cases/managed_stand/standalone.rs::a_debug_server_that_does_not_start_stops_the_server
  - src/use_cases/managed_stand/standalone.rs::a_taken_debug_port_moves_the_block_only_with_the_debug_server
  - src/use_cases/managed_stand/standalone.rs::a_debug_server_left_by_a_fallen_server_is_stopped_before_the_start
  - src/platform/ibsrv.rs::a_debug_server_registers_its_targets_on_the_given_debugger
---

# Сервер отладки автономного сервера остаётся включённым до `--no-debug`

`standalone start --debug` и `standalone restart --debug` поднимают `ibsrv` с ключами `--debug=http
--debug-server-url=http://127.0.0.1:<порт>`, а следом — свой `dbgs` раннера из той же установки на этом порту с
`--ownerPID` pid `ibsrv`: свой сервер отладки `ibsrv` (`--debug-port`) внешнему отладчику протокола `rdbg` отвечает
406, а цели на внешнем `dbgs` регистрирует (замер #759). Сервер готов, когда `dbgs` ответил по `rdbg`; не ответил —
`ibsrv` гасится, закреплённое не меняется.

Порт отладки — следом за диапазоном прямых соединений, внутри шага закреплённого блока. При включённой отладке он
проверяется вместе с блоком, и занятый чужим процессом порт переносит блок целиком; без отладки он не проверяется.
Признак и имя базы закрепляются вместе с блоком после успешного запуска и держатся для следующих `start` и
`restart`, пока `--no-debug` не снимет признак.

| Сервер | Ключ | `start` | `restart` |
| --- | --- | --- | --- |
| не работает | любой | поднимает с ключом, без ключа — как закреплено | то же |
| работает, отладка совпадает или ключа нет | — | «уже работает» | перезапуск `ibsrv` и `dbgs` |
| работает, отладка не совпадает | `--debug` / `--no-debug` | отказ с `standalone restart <ключ>` | перезапуск с ключом |
| работает с отладкой, своего `dbgs` нет | — | отказ с `standalone restart` | перезапуск |

`dbgs` выходит вместе с `ibsrv`; `standalone stop` и `restart` дожидаются его конца и называют его pid в
`stopped_debug_pid`. `dbgs` упавшего сервера следующий `start` дожидается или гасит по записи, не считая его порт
занятым чужим процессом. PostgreSQL шага `restart` не трогает. Управляемый автономный сервер есть только на Windows
(`INV.CLI.MANAGED-SERVICES-RUN-ON-WINDOWS`).
