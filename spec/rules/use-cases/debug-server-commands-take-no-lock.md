---
id: INV.USE-CASES.DEBUG-SERVER-COMMANDS-TAKE-NO-LOCK
check:
  - tests/cli_debug_server.rs::stop_answers_while_another_command_holds_the_work_path
  - tests/cli_debug_server.rs::status_reports_an_unknown_session_without_waiting_for_the_work_path
  - tests/cli_debug_server.rs::prepare_refuses_an_explicit_external_cluster_debugger_without_falling_back
---

# Команды сервера отладки не берут замок рабочей копии

`debug-server start`, `debug-server status` и `debug-server stop` работают
без замка `workPath` и замка стенда. ИБ они не открывают. Создание и
освобождение аренды синхронизируются собственным замком процесса в
`workPath/debug-server`. `status` наблюдает записи и готовность сервера
без этого замка.
