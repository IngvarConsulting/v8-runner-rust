---
id: CTR.WIRE.EXTENSIONS-DATA
version: 2
artifact: docs/schemas/command-data/extensions.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it
---

# `data` изменения состава расширений

Этой формой отвечает всё, что меняет состав расширений в базе: `extensions` без
подкоманды, `create`, `delete`, `activate`. Шаг называет расширение и действие над ним,
поэтому один вызов над несколькими расширениями отчитывается по каждому отдельно.

Чтение состава отвечает другой формой — [`CTR.WIRE.EXTENSIONS-INVENTORY-DATA`](extensions-inventory-data.md):
у чтения есть список расширений, которого у изменения нет.

## Пример

```json
{
  "provider": {"selected": "ibcmd", "origin": {"kind": "default"}},
  "ok": true,
  "provider_dispatched": false,
  "duration_ms": 0,
  "steps": [
    {
      "target": "Demo",
      "action": "create",
      "ok": true,
      "message": "would create 'Demo' in file infobase 'build/ib' with no configured infobase user via /opt/1cv8/bin/ibcmd",
      "duration_ms": 0
    }
  ]
}
```
