---
id: INV.USE-CASES.AN-IBCMD-RUN-HOLDS-THE-DATA-DIRECTORY-OF-THE-PROJECT
check:
  - tests/architecture_guardrails.rs::an_ibcmd_process_is_built_only_by_those_who_give_it_a_data_directory
  - src/use_cases/apply/act.rs::an_ibcmd_apply_holds_the_data_directory_of_the_project_as_a_critical_phase
  - src/use_cases/reset/act.rs::an_ibcmd_rollback_holds_the_data_directory_of_the_project_as_a_critical_phase
  - tests/cli_init.rs::an_ibcmd_create_of_a_file_base_holds_the_data_directory_of_the_project
  - tests/cli_status.rs::an_ibcmd_status_deep_holds_the_data_directory_of_the_project
---

# `ibcmd` к базе проекта держит каталог данных проекта

Запуск `ibcmd` к базе проекта передаёт `--data <workPath>/ibcmd-data`. Умолчательный каталог
данных один на пользователя и блокируется целиком, поэтому без своего каталога запуски в разных
проектах отказывают друг другу, хотя базы у них разные. Внутри проекта каталог никто не
оспаривает: такие команды идут под блокировкой `workPath`.
