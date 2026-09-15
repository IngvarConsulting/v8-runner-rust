---
id: CTR.WIRE.EXTENSIONS-DATA
status: active
governs: product
version: 2
decision: DEC.2026-09-14.RECEIPT-EXPLAINS-PROVIDER-CHOICE
artifact: docs/schemas/command-data/extensions.schema.json
producer: src/domain/extensions.rs
consumers: [cli, unica]
check: [src/command_data.rs::generated_command_data_schemas_are_current, tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it]
scope: [wire, cli]
---

# `data` изменения состава расширений

Этой формой отвечает всё, что меняет состав расширений в базе: `extensions` без
подкоманды, `create`, `delete`, `activate`. Шаг называет расширение и действие над ним,
поэтому один вызов над несколькими расширениями отчитывается по каждому отдельно.

Чтение состава отвечает другой формой — [`CTR.WIRE.EXTENSIONS-INVENTORY-DATA`](CTR.WIRE.EXTENSIONS-INVENTORY-DATA.md):
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
