---
id: CTR.WIRE.INIT-DATA
status: active
governs: product
version: 2
decision: DEC.2026-09-14.RECEIPT-EXPLAINS-PROVIDER-CHOICE
artifact: docs/schemas/command-data/init.schema.json
producer: src/domain/init.rs
consumers: [cli, unica]
check: [src/command_data.rs::generated_command_data_schemas_are_current, tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it]
scope: [wire, cli]
---

# `data` команды `init`

Команда доводит окружение до состояния, в котором можно собирать: создаёт базу и, для
формата EDT, рабочее пространство. Форма перечисляет шаги со статусом каждого, поэтому
пропущенный шаг виден так же явно, как сделанный, и не притворяется успехом.

`provider_dispatched: false` означает превью: план построен, платформа найдена, но ничего
не создано.

## Пример

```json
{
  "provider": {"selected": "designer", "origin": {"kind": "default"}},
  "ok": true,
  "provider_dispatched": false,
  "duration_ms": 0,
  "steps": [
    {
      "target": "infobase",
      "action": "create",
      "status": "planned",
      "message": "would create a file infobase at 'build/ib' via /opt/1cv8/bin/1cv8",
      "duration_ms": 0
    },
    {
      "target": "edt_workspace",
      "action": "import",
      "status": "skipped",
      "message": "EDT workspace initialization is not applicable for format=DESIGNER",
      "duration_ms": 0
    }
  ]
}
```
