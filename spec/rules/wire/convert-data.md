---
id: CTR.WIRE.CONVERT-DATA
version: 1
artifact: docs/schemas/command-data/convert.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it
---

# `data` команды `convert`

Перевод исходников между форматами EDT и Designer отчитывается направлением, охватом и
списком того, что получилось на выходе. Направление в ответе обязательно: команда
выбирает его из формата проекта, и вызывающий узнаёт выбор отсюда, а не из своих
предположений.

## Пример

```json
{
  "ok": true,
  "provider_dispatched": true,
  "direction": "DESIGNER_TO_EDT",
  "scope": "ALL",
  "workspace_path": "build/convert/edt-workspace",
  "outputs": [
    {
      "source_set": "main",
      "source_path": "src/cf",
      "target_path": "build/convert/out/main/edt"
    }
  ],
  "duration_ms": 4210
}
```
