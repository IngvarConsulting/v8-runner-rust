---
id: INV.CLI.PROVIDER-IS-NOT-A-FLAG
check:
  - tests/provider_matrix.rs::no_command_accepts_a_provider_flag
  - tests/provider_matrix.rs::no_mcp_tool_takes_a_provider_field
---

# Провайдера нельзя выбрать флагом или полем вызова

Ни у одной команды нет публичного аргумента выбора исполнителя, и ни один инструмент MCP такого поля не принимает.
