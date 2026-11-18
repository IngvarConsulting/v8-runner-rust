---
id: INV.PLATFORM.A-DEBUG-SERVER-DIES-WITH-ITS-OWNER
check:
  - src/platform/dbgs.rs::the_request_listens_on_the_loopback_and_names_its_owner
  - src/use_cases/managed_stand/debug_server.rs::a_dead_owner_is_refused_before_anything_starts
  - src/use_cases/managed_stand/debug_server.rs::the_runner_itself_is_no_owner
  - src/use_cases/managed_stand/debug_server.rs::start_stops_the_server_of_an_interrupted_start
  - src/use_cases/managed_stand/debug_server.rs::start_sweeps_sessions_whose_server_is_gone
---

# Сервер отладки раннера живёт не дольше владельца

`debug-server start --owner-pid <pid>` передаёт процесс отладчика в `dbgs --ownerPID`: с его концом сервер выходит
сам, даже если отладчик упал и не позвал `debug-server stop`. Pid, которого нет, и pid самого раннера — отказ до
запуска: иначе сервер вышел бы сразу после ответа. Без `--owner-pid` сервер живёт до `debug-server stop`. Сеансы, чей сервер уже
вышел, убирает следующий `debug-server start`. Он же гасит `dbgs` оборванного запуска — запускавший раннер
убит, и сеанса никто не получил.

`dbgs` автономного сервера запускается с `--ownerPID` pid своего `ibsrv` и выходит вместе с ним.

Сервер — и сеанса, и автономного сервера — слушает только петлю `127.0.0.1` и запускается без пароля: любой процесс этой машины может к нему
подключиться. Это принятый риск: отладка идёт на машине разработчика, а адрес отдаётся только вызывающему.
