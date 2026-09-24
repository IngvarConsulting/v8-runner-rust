---
id: CTR.MCP.PUBLISHED-TOOL-SURFACE
version: 2
artifact: docs/schemas/mcp-tools.json
check: [tests/mcp_http.rs::mcp_http_initialize_reuses_session_and_lists_tools]
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

Сторож
`tests/architecture_guardrails.rs::mcp_surface_snapshot_stays_explicit_and_documented`
сверяет три перечня между собой: этот, `src/mcp/server.rs` и свой собственный
`EXPECTED_MCP_TOOLS`. Расхождение любой пары валит проверку, и числительное прозы он
сверяет со счётом строк, а не принимает за якорь.

Прочие места смена состава задевает тоже, и их держит автор: `src/mcp/request.rs`,
`src/mcp/service.rs`, `docs/CAPABILITIES.md` и `README.md`.
Машиночитаемая часть ответа меняется вместе с `src/command_envelope.rs`, а схема конверта
порождается командой `UPDATE_ENVELOPE_SCHEMA=1 cargo test
generated_envelope_schema_is_current` и руками не правится.

Сценарий, доступный только в командной строке, не публикуется инструментом MCP по
умолчанию: наличие команды доступности инструмента не означает.

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
