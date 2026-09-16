---
id: CTR.MCP.PUBLISHED-TOOL-SURFACE
status: active
governs: product
version: 2
decision: DEC.2026-09-16.A-THIN-CLIENT-OPENS-EITHER-ADDRESS
artifact: docs/schemas/mcp-tools.json
producer: src/mcp/service.rs
consumers: [mcp, docs]
check: tests/mcp_http.rs::mcp_http_initialize_reuses_session_and_lists_tools
scope: [mcp, wire]
---

# Состав опубликованных инструментов MCP

Перечень инструментов — наблюдаемая форма: клиент видит именно его. Форма закреплена артефактом `docs/schemas/mcp-tools.json`: имя каждого инструмента и
его схема входа целиком. Проверка сверяет с ним ответ живого сервера, поэтому
добавленное обязательное поле ломает её при неизменном имени. Артефакт порождается
командой `UPDATE_MCP_SURFACE=1 cargo test --test mcp_http`, а не правится руками.

Опубликованы восемь инструментов:

- `run_all_tests`
- `run_module_tests`
- `build_project`
- `dump_config`
- `launch_app`
- `check_syntax_edt`
- `check_syntax_designer_config`
- `check_syntax_designer_modules`

Состав меняется только вместе с версией этой формы.

## Пример

Фрагмент закреплённой поверхности — схема входа одного инструмента:

```json
{
  "tools": {
    "dump_config": {
      "title": "McpDumpConfigRequest",
      "type": "object",
      "description": "MCP request for `dump_config`.",
      "properties": {
        "mode": {
          "default": null,
          "description": "Dump mode, for example FULL or INCREMENTAL.",
          "type": ["string", "null"]
        }
      }
    }
  }
}
```
