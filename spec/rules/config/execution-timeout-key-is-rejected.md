---
id: INV.CONFIG.EXECUTION-TIMEOUT-KEY-IS-REJECTED
check:
  - src/config/loader.rs::load_config_refuses_the_retired_execution_timeout_key_by_name
  - src/config/loader.rs::local_overlay_refuses_the_retired_execution_timeout_key_by_name_too
---

# Снятый ключ срока команды отклоняется по имени

Ключ верхнего уровня `execution_timeout` не принимают ни проектный файл, ни местный слой.
Отказ называет ключ, говорит, что у команды нет срока, и показывает, где предел остаётся: у
шага (`tools.edt_cli.command_timeout_ms` и другие) и у ожидания допуска MCP
(`mcp.execution.admission_timeout_ms`).
