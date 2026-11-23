---
id: INV.CLI.MANAGED-SERVICES-RUN-ON-WINDOWS-AND-LINUX
check:
  - src/use_cases/managed_stand/mod.rs::other_systems_refuse_start_and_restart_without_starting_anything
  - tests/cli_dbms.rs::other_systems_refuse_start_as_the_environment
  - tests/cli_dbms.rs::linux_start_reaches_postgres_build_discovery
  - src/use_cases/managed_stand/cluster.rs::other_systems_refuse_the_cluster
  - src/use_cases/managed_stand/standalone.rs::linux_refuses_the_standalone_server_before_any_process
  - src/use_cases/managed_stand/standalone.rs::linux_refuses_the_debug_server_of_the_standalone_server
  - src/use_cases/managed_stand/cluster.rs::under_root_no_service_of_the_stand_starts
---

# Службы стенда по ОС

На Windows раннер запускает `dbms`, `cluster`, `web` и `standalone`. На Linux `dbms`, `cluster` и
`web` запускает обычный пользователь: от root их `start` и `publish` управляемой базы отказывают.
`standalone start|restart` на Linux и запуск этих служб на остальных платформах отвечают
`environment_unavailable` до запуска процессов.

Сервер отладки `debug-server` службой стенда не является и раннер его не ограничивает по ОС. Свой `dbgs`
раннера для файловой базы вживую замерен только на Windows; на Linux и macOS он не замерен (#778). Сервер
отладки кластера (`cluster start --debug`) есть там же, где управляемый кластер, — на Windows и Linux (Linux
замерен в #768); на остальных платформах `--debug` отвечает `environment_unavailable`. Сервер отладки
автономного сервера — только на Windows: `debug-server start` у его базы на других системах отвечает
`environment_unavailable`, как `standalone start`.
