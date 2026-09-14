---
id: CTR.WIRE.COMMAND-ENVELOPE
status: active
governs: product
version: 1
decision: DEC.2026-04-20.JSON-MESSAGE-IS-THE-ONLY-FORMAT-SWITCH
artifact: docs/schemas/command-envelope.schema.json
producer: src/command_envelope.rs
consumers: [cli, mcp, ci]
check: tests/contract_envelope.rs::a_successful_command_answers_in_the_pinned_envelope_form
scope: [wire, cli]
---

# Конверт ответа команды

Структурный ответ любой команды имеет одну форму, закреплённую схемой
`docs/schemas/command-envelope.schema.json`: `ok`, `command`, `duration_ms`, `data`,
`warnings`, `steps` и необязательная `error` из `code`, `kind`, `message`. Список полей
конверта и шага закрыт: новое поле у любого из них валит проверку.

Поле `data` принадлежит команде, и этой формой не описывается: у каждой команды свой
предмет. Часть команд кладёт свои шаги внутрь `data`, и те под эту схему не подпадают.
