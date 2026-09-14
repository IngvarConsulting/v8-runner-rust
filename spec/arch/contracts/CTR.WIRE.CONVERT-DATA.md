---
id: CTR.WIRE.CONVERT-DATA
status: active
governs: product
version: 1
decision: DEC.2026-09-14.EVERY-COMMAND-PINS-THE-FORM-OF-ITS-DATA
artifact: docs/schemas/command-data/convert.schema.json
producer: src/domain/convert.rs
consumers: [cli, unica]
check: [src/command_data.rs::generated_command_data_schemas_are_current, tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it]
scope: [wire, cli]
---

# `data` команды `convert`

Перевод исходников между форматами EDT и Designer отчитывается направлением, охватом и
списком того, что получилось на выходе. Направление в ответе обязательно: команда
выбирает его из формата проекта, и вызывающий узнаёт выбор отсюда, а не из своих
предположений.

## Пример

```json
{
  "ok": false,
  "provider_dispatched": true,
  "direction": "DESIGNER_TO_EDT",
  "scope": "ALL",
  "workspace_path": "build/convert/edt-workspace",
  "outputs": [],
  "duration_ms": 0,
  "message": "platform error: utility '1cedtcli' was not found"
}
```
