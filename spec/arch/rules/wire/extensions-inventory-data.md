---
id: CTR.WIRE.EXTENSIONS-INVENTORY-DATA
version: 3
artifact: docs/schemas/command-data/extensions-inventory.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it
---

# `data` чтения состава расширений

Так отвечают `extensions list` и `extensions info`: составом установленных расширений в
том порядке, в каком его назвала платформа. Порядок не документирован и не является
порядком создания — опираться на него нельзя, и форма этого не обещает.

Чтение состава — действие, а не взгляд: платформа стартует, сессия открывается, в журнале
остаётся след. Поэтому у него тоже есть превью, и в нём `extensions` пуст: ничего не
спрашивали.

Предмет чтения назван полем `requested` — `{"kind": "all"}` или
`{"kind": "named", "name": …}` — и в превью, и в ответе: вызывающий сверяет ответ со
своим запросом по нему, а не по формулировке `plan`. `plan` остаётся для человека.

## Пример

```json
{
  "provider": {"selected": "ibcmd", "origin": {"kind": "default"}},
  "ok": true,
  "provider_dispatched": false,
  "requested": {"kind": "all"},
  "plan": "would read every installed extension of file infobase 'build/ib' with no configured infobase user via /opt/1cv8/bin/ibcmd",
  "extensions": [],
  "duration_ms": 0
}
```
