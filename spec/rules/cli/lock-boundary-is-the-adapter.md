---
id: INV.CLI.LOCK-BOUNDARY-IS-THE-ADAPTER
check:
  - tests/architecture_guardrails.rs::every_scenario_is_dispatched_under_the_workspace_lock
  - tests/cli_bootstrap.rs::clone_refuses_a_busy_workspace_before_writing_the_project
  - tests/mcp_stdio.rs::mcp_stdio_edt_syntax_refuses_a_busy_workspace
  - tests/mcp_stdio.rs::mcp_stdio_a_second_edt_syntax_call_on_a_busy_workspace_is_refused_at_once
---

# Блокировку берёт адаптер команды, а не сценарий

Публичная команда, работающая с состоянием под `workPath`, захватывает блокировку на границе адаптера; вложенные шаги идут под ней.
