---
id: INV.PLATFORM.AGENT-SESSION-LIVES-WITH-THE-LOCK
check:
  - tests/cli_build_agent.rs::a_managed_build_loads_and_updates_in_one_session_and_records_the_generation
  - tests/cli_build_agent.rs::a_build_without_changes_opens_no_session
  - tests/architecture_guardrails.rs::every_scenario_is_dispatched_under_the_workspace_lock
---

# Агентская сессия не живёт дольше замка

Исполнитель `agent` открывает сессию под блокировкой рабочего каталога и только когда агенту
есть что поручить — сборка без изменений сессии не открывает, — и закрывает её раньше, чем
блокировка освобождается. Вложенная оркестрация идёт в той же сессии: у сборки загрузка
исходников и обновление конфигурации базы — два шага одного разговора, а не два процесса.

У долгоживущего MCP-сервера между вызовами живёт сессия EDT; агентская по-прежнему
открывается под замок конкретной команды.
