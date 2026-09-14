---
id: CTR.WIRE.MCP-REFUSAL-DATA
status: active
governs: product
version: 1
decision: DEC.2026-09-14.EVERY-COMMAND-PINS-THE-FORM-OF-ITS-DATA
artifact: docs/schemas/command-data/mcp-refusal.schema.json
producer: src/mcp/service.rs
consumers: [mcp]
check: [src/command_data.rs::generated_command_data_schemas_are_current, tests/mcp_stdio.rs::mcp_stdio_returns_structured_business_failure]
scope: [wire, mcp]
---

# `data` отказа адаптера MCP

Отказ на стороне MCP несёт больше, чем отказ CLI: адаптер называет инструмент, по которому
пришёл вызов, и — когда знает — поле входа, из-за которого вызов отклонён. По ним клиент
чинит свой вызов сам, не спрашивая человека.

Форма общая для всех инструментов и объявлена отдельно от форм команд: она приходит
вместо предмета, а не вместе с ним.

## Пример

```json
{
  "message": "module name must not be blank",
  "tool": "run_module_tests",
  "field": "module_name"
}
```
