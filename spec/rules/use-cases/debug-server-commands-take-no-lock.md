---
id: INV.USE-CASES.DEBUG-SERVER-COMMANDS-TAKE-NO-LOCK
check:
  - tests/cli_debug_server.rs::stop_answers_while_another_command_holds_the_work_path
  - tests/cli_debug_server.rs::status_reports_an_unknown_session_without_waiting_for_the_work_path
  - tests/cli_debug_server.rs::prepare_refuses_an_explicit_external_cluster_debugger_without_falling_back
---

# Команды сервера отладки не берут замок рабочей копии

`debug-server start`, `debug-server prepare`, `debug-server status` и `debug-server stop` не ждут замка `workPath` и замка стенда: отладчик гасит сервер при
отсоединении, даже если в это время идёт `push`. Базу они не открывают. Гонку команд одного сеанса решает его
запись: сеанс, который прямо сейчас запускает живая команда, `stop` не трогает и отказывает `workspace_busy`.

`start --ensure` и остановка созданного им сеанса синхронизируются замком этого
сеанса в `workPath/debug-server`. Замок `workPath` и замок стенда по-прежнему
независимы от запуска, восстановления и остановки отладки. Аренды `prepare`
синхронизируются замком своего процесса в том же каталоге. Остановка одной
аренды проверяет её принадлежность и не ждёт подготовки другой области.
