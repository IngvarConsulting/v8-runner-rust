---
id: INV.CLI.MANAGED-SERVICES-RUN-ON-WINDOWS-AND-LINUX
check:
  - src/use_cases/managed_stand/mod.rs::other_systems_refuse_start_and_restart_without_starting_anything
  - tests/cli_dbms.rs::other_systems_refuse_start_as_the_environment
  - tests/cli_dbms.rs::linux_start_reaches_postgres_build_discovery
  - src/use_cases/managed_stand/cluster.rs::other_systems_refuse_the_cluster
  - src/use_cases/managed_stand/standalone.rs::linux_refuses_the_standalone_server_before_any_process
  - src/use_cases/managed_stand/cluster.rs::under_root_no_service_of_the_stand_starts
---

# Службы стенда по ОС

На Windows раннер запускает `dbms`, `cluster`, `web` и `standalone`. На Linux `dbms`, `cluster` и
`web` запускает обычный пользователь: от root их `start` и `publish` управляемой базы отказывают.
`standalone start|restart` на Linux и запуск этих служб на остальных платформах отвечают
`environment_unavailable` до запуска процессов.

Подготовка собственного `dbgs` командой `debug-server start` использует
проверку владения адресом и ответ RDBG на Windows, Linux и macOS arm64.
Кластер запускает свой сервер отладки через `cluster start --debug` на Windows
и Linux. На Windows `debug-server start` может подтвердить принадлежность
кластерного `dbgs` выбранной регистрации. Управляемый автономный сервер
публикует адрес в `standalone status`.

`debug-server status` проверяет процесс и адрес собственного `dbgs`
по аренде, см. `CTR.WIRE.DEBUG-SERVER-STATUS-DATA`.
