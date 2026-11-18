---
id: INV.PLATFORM.A-DEBUG-SERVER-DIES-WITH-ITS-OWNER
check:
  - src/platform/dbgs.rs::the_request_listens_on_the_loopback_and_names_its_owner
  - src/use_cases/managed_stand/debug_server.rs::a_dead_owner_is_refused_before_anything_starts
  - src/use_cases/managed_stand/debug_server.rs::start_sweeps_sessions_whose_server_is_gone
---

# Сервер отладки раннера живёт не дольше владельца

`debug-server start --owner-pid <pid>` передаёт процесс отладчика в `dbgs --ownerPID`: с его концом сервер выходит
сам, даже если отладчик упал и не позвал `debug-server stop`. Мёртвый или чужой pid — отказ до запуска: иначе
сервер вышел бы сразу после ответа. Без `--owner-pid` сервер живёт до `debug-server stop`. Сеансы, чей сервер уже
вышел, убирает следующий `debug-server start`.

Сервер слушает только петлю `127.0.0.1` и запускается без пароля: любой процесс этой машины может к нему
подключиться. Это принятый риск: отладка идёт на машине разработчика, а адрес отдаётся только вызывающему.
