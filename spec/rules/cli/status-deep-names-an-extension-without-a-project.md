---
id: INV.CLI.STATUS-DEEP-NAMES-AN-EXTENSION-WITHOUT-A-PROJECT
check:
  - tests/cli_status.rs::status_deep_names_an_extension_without_a_project
  - tests/cli_status.rs::status_deep_matches_extension_names_without_case
  - tests/cli_status.rs::status_deep_marks_the_client_mcp_tool_extension
---

# `status --deep` называет расширение базы без набора в проекте

`status --deep` читает состав расширений базы и сопоставляет его с наборами расширений
проекта по имени, которым набор называет расширение платформе, без учёта регистра — и у
латиницы, и у кириллицы. Расширение базы без набора отвечает `source_set: null`, набор
проекта без расширения в базе стоит в `missing_in_base`. Расширение-инструмент клиентского
MCP (`tools.client_mcp.extension`) набором не объявляется
(`INV.CLI.PULL-ALL-DOES-NOT-DECLARE-THE-CLIENT-MCP-TOOL-EXTENSION`) и отвечает `tool: true`:
за расширение без проекта оно не выдаётся.

Источник: [`cli.html`](../../../docs/site/cli.html), раздел «У расширения в базе без проекта
есть имя».
