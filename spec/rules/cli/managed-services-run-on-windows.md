---
id: INV.CLI.MANAGED-SERVICES-RUN-ON-WINDOWS
check:
  - src/use_cases/managed_stand/mod.rs::other_systems_refuse_start_and_restart_without_starting_anything
  - tests/cli_dbms.rs::other_systems_refuse_start_as_the_environment
  - src/use_cases/managed_stand/cluster.rs::other_systems_refuse_the_cluster
---

# Службы стенда раннер запускает только на Windows

На Linux и macOS `dbms start`, `cluster start`, `standalone start` и их `restart` отвечают
`environment_unavailable`, не запуская процессов: поведение служб замерено только на Windows.
