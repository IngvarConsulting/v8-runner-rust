---
id: CTR.WIRE.BUILD-DATA
status: active
governs: product
version: 2
decision: DEC.2026-09-14.RECEIPT-EXPLAINS-PROVIDER-CHOICE
artifact: docs/schemas/command-data/build.schema.json
producer: src/domain/build.rs
consumers: [cli, mcp, unica]
check: [src/command_data.rs::generated_command_data_schemas_are_current, tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it]
scope: [wire, cli, mcp]
---

# `data` команды `build`

Сборка идёт по наборам исходников, и форма отчитывается по каждому: какой набор, каким
режимом загружен и почему. Режим выбирают правила частичной загрузки, а не вызывающий,
поэтому `mode` в ответе — решение раннера, и в нём же его причина.

Инструмент MCP `build_project` отвечает этой же формой: поверхность MCP не повторяет CLI,
но предмет команды у них один.

## Пример

```json
{
  "provider": {"selected": "designer", "origin": {"kind": "default"}},
  "ok": true,
  "provider_dispatched": false,
  "duration_ms": 1,
  "steps": [
    {
      "source_set": "main",
      "mode": "full",
      "ok": true,
      "message": "full load selected by partial-load rules; planned, Designer not dispatched",
      "duration_ms": 0
    }
  ]
}
```
