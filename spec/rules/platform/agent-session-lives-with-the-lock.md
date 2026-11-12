---
id: INV.PLATFORM.AGENT-SESSION-LIVES-WITH-THE-LOCK
check:
  - tests/cli_build_agent.rs::a_managed_build_loads_and_updates_in_one_session_and_records_the_generation
  - tests/cli_build_agent.rs::a_build_without_changes_opens_no_session
  - tests/cli_build_agent.rs::a_push_with_nothing_to_load_leaves_the_early_agent_connected_without_work
  - tests/architecture_guardrails.rs::every_scenario_is_dispatched_under_the_workspace_lock
---

# Агентская сессия не живёт дольше замка

Исполнитель `agent` открывает сессию под блокировкой рабочего каталога и только когда агенту
есть что поручить — сборка без изменений сессии не открывает, — и закрывает её раньше, чем
блокировка освобождается. `pull` с живым агентом открывает сессию в начале команды, пока
готовится (`INV.PLATFORM.A-LIVE-AGENT-STARTS-WHILE-THE-COMMAND-PREPARES`): отказ подготовки
закрывает её, не дав агенту работы; `push` подключает агента, только когда разбор нашёл
работу, а подключает его к базе, пока идёт разбор, — кроме отправки, у набора которой
поколение до загрузки сверяет инструмент записи своим процессом: тогда агента подключает сессия
после сверки (`INV.PLATFORM.A-LIVE-AGENT-STARTS-WHILE-THE-COMMAND-PREPARES`). Вложенная оркестрация идёт в той же сессии:
у сборки загрузка исходников и обновление конфигурации базы — два шага одного разговора, а не
два процесса.

Сессия команды — это её разговор с агентом у сторожа (`begin` … `end`). Процесс живого агента,
его сессию SSH и его подключение к базе держит сторож, и они живут дольше замка
(`INV.PLATFORM.A-MANAGED-AGENT-OUTLIVES-THE-COMMAND`,
`INV.PLATFORM.A-LIVE-AGENT-STAYS-CONNECTED-BETWEEN-COMMANDS`). У разового агента сессия команды
и есть его подключение к базе (`common connect-ib` … `common disconnect-ib`).

У долгоживущего MCP-сервера между вызовами живёт сессия EDT; агентская по-прежнему
открывается под замок конкретной команды.
