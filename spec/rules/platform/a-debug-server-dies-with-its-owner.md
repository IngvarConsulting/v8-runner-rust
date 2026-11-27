---
id: INV.PLATFORM.A-DEBUG-SERVER-DIES-WITH-ITS-OWNER
check:
  - src/platform/dbgs.rs::the_request_listens_on_the_loopback_and_names_its_owner
  - src/use_cases/managed_stand/debug_server.rs::debug_start_rejects_a_dead_owner_and_the_runner_itself_before_work
  - src/use_cases/managed_stand/debug_server.rs::prepare_cancellation_stops_only_the_unpublished_owned_attempt
---

# Сервер отладки раннера живёт не дольше владельца

`debug-server start --owner-pid <pid>` передаёт процесс отладчика в `dbgs --ownerPID`: с его концом сервер выходит
сам, даже если отладчик упал и не позвал `debug-server stop`. Для собственного сервера раннер перед запуском проверяет PID и время старта владельца. Несуществующий PID и PID самого раннера вызывают отказ. Отмена до выдачи аренды завершает только процесс текущей попытки.

`dbgs` автономного сервера запускается с `--ownerPID` pid своего `ibsrv` и выходит вместе с ним.

Сервер — и сеанса, и автономного сервера — слушает только петлю `127.0.0.1` и запускается без пароля: любой процесс этой машины может к нему
подключиться. Это принятый риск: отладка идёт на машине разработчика, а адрес отдаётся только вызывающему.
