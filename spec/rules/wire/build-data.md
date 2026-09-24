---
id: CTR.WIRE.BUILD-DATA
version: 2
artifact: docs/schemas/command-data/push.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it
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
