---
id: CTR.WIRE.COMMAND-ENVELOPE
status: active
governs: product
version: 1
decision: DEC.2026-09-14.EVERY-COMMAND-PINS-THE-FORM-OF-ITS-DATA
artifact: docs/schemas/command-envelope.schema.json
producer: src/command_envelope.rs
consumers: [cli, mcp, ci]
check: tests/contract_envelope.rs::a_successful_command_answers_in_the_pinned_envelope_form
scope: [wire, cli]
---

# Конверт ответа команды

Структурный ответ любой команды имеет одну оболочку, закреплённую схемой
`docs/schemas/command-envelope.schema.json`: `ok`, `command`, `duration_ms`, `data`,
`warnings`, `steps` и необязательная `error` из `code`, `kind`, `message`. Список полей
конверта и шага закрыт: новое поле у любого из них валит проверку.

Предмет команды лежит в `data`, и этой формой он не описан: у каждой команды он свой.
Форму `data` закрепляет отдельный контракт на каждую форму — перечень объявлен в
`docs/schemas/command-data/index.json` и разобран по записям `CTR.WIRE.*-DATA`.
Единственное, что здесь обещано про `data`, — что поле есть всегда. Часть команд кладёт
свои шаги внутрь `data`, и те под эту схему не подпадают.

## Пример

```json
{
  "ok": true,
  "command": "infobase.configuration.export",
  "duration_ms": 8,
  "data": {},
  "warnings": [],
  "steps": [
    {
      "name": "resolve-provider",
      "ok": true,
      "status": "succeeded",
      "kind": "planning",
      "duration_ms": 3,
      "message": "designer selected: argv tested"
    }
  ]
}
```
