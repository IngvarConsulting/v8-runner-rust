---
id: INV.USE-CASES.DEBUG-SERVER-COMMANDS-TAKE-NO-LOCK
check:
  - tests/cli_debug_server.rs::stop_answers_while_another_command_holds_the_work_path
---

# Команды сервера отладки не берут замок рабочей копии

`debug-server start` и `debug-server stop` не ждут замка `workPath` и замка стенда: отладчик гасит сервер при
отсоединении, даже если в это время идёт `push`. Базу они не открывают. Гонку команд одного сеанса решает его
запись: сеанс, который прямо сейчас запускает живая команда, `stop` не трогает и отказывает `workspace_busy`.
