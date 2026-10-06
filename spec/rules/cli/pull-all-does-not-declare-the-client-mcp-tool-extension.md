---
id: INV.CLI.PULL-ALL-DOES-NOT-DECLARE-THE-CLIENT-MCP-TOOL-EXTENSION
check:
  - tests/cli_pull_all.rs::the_client_mcp_tool_extension_is_not_declared_and_the_project_stays_valid
  - src/use_cases/dump_config/all.rs::the_client_mcp_tool_extension_is_not_declared
---

# Расширение-инструмент набором не объявляется

Расширение `tools.client_mcp.extension` раннер ставит в базу сам. `pull --all` его не
выгружает и набором не объявляет, в каком бы регистре база ни назвала его, — иначе
следующий запуск отказал бы на проверке конфигурации.
