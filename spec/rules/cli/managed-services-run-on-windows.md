---
id: INV.CLI.MANAGED-SERVICES-RUN-ON-WINDOWS
check:
  - src/use_cases/managed_stand/mod.rs::other_systems_refuse_start_and_restart_without_starting_anything
  - tests/cli_dbms.rs::other_systems_refuse_start_as_the_environment
  - tests/cli_dbms.rs::linux_start_reaches_postgres_build_discovery
  - src/use_cases/managed_stand/cluster.rs::other_systems_refuse_the_cluster
---

# Службы стенда по ОС

На Windows раннер запускает `dbms`, `cluster` и `standalone`. На Linux `dbms` и `cluster` запускает
обычный пользователь. `standalone start|restart` на Linux и запуск этих служб на остальных
платформах отвечают `environment_unavailable` до запуска процессов.
