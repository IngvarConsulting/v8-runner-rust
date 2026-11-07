---
id: INV.USE-CASES.A-WORKING-COMMAND-DOES-NOT-START-A-SERVICE
check:
  - tests/cli_standalone_managed.rs::a_working_command_without_the_server_is_refused_and_names_the_next_command
  - tests/cli_standalone_managed.rs::start_without_an_infobase_names_infobase_create
---

# Рабочая команда службу не поднимает

Команда, которой нужно открыть управляемую базу, при неподнятой службе отказывает до запуска платформы и
называет следующую команду. Команды, которые базу не открывают, — `make`, `convert` без базы, `status` без
`--deep`, — от службы не зависят. Совет не ведёт в новый отказ: у ещё не созданной базы он называет
`infobase create`.
