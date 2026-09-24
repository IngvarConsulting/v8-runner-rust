---
id: INV.USE-CASES.AN-IBCMD-CONNECTION-IS-BUILT-ONLY-FOR-IBCMD
check:
  - tests/cli_agent_scenarios.rs::a_server_base_without_dbms_serves_extensions_through_the_agent
  - tests/architecture_guardrails.rs::an_ibcmd_connection_is_built_only_where_ibcmd_runs
---

# Подключение `ibcmd` строят только для `ibcmd`

Сценарий строит подключение `ibcmd` — а с ним требует секцию `infobase.dbms` у серверной
базы — только когда вызывает `ibcmd`: исполнителем или для пробы. Выбранный агент или
Конфигуратор, которому `ibcmd` не нужен, подключения не строит и из-за отсутствия секции не
отказывает.
