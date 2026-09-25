---
id: INV.MCP.STDIO-STDOUT-CARRIES-ONLY-PROTOCOL-FRAMES
check:
  - tests/mcp_stdio.rs::mcp_stdio_stdout_carries_only_protocol_frames
  - tests/mcp_stdio.rs::mcp_missing_config_reports_error_on_stderr
  - tests/cli_global_flags.rs::the_server_keeps_stdout_for_the_protocol_when_it_refuses_the_preview_key
  - tests/cli_bootstrap.rs::mcp_rejects_clean_before_execution_flag
---

# stdout сервера MCP по stdio несёт только кадры протокола

Сервер MCP по stdio пишет в stdout только кадры JSON-RPC: отказ при запуске уходит в stderr, а
предупреждения загрузки конфигурации — в журнал действий.
