---
id: CTR.WIRE.REFUSAL-DATA
status: active
governs: product
version: 1
decision: DEC.2026-09-14.EVERY-COMMAND-PINS-THE-FORM-OF-ITS-DATA
artifact: docs/schemas/command-data/refusal.schema.json
producer: src/cli/output.rs
consumers: [cli, unica]
check: [src/command_data.rs::generated_command_data_schemas_are_current, tests/contract_command_data.rs::a_refusal_before_dispatch_answers_in_the_shared_form]
scope: [wire, cli]
---

# `data` отказа до диспетчеризации

Эту форму печатает любая команда, отказавшая до того, как начала работу: вход не прошёл
проверку, платформа не запускалась, плана нет. Предмета у такого ответа нет, поэтому в
`data` только текст отказа — машинная часть причины живёт в `error` конверта.

Форма общая для всех команд, и в перечне форм она объявлена отдельно от них: клиент
разбирает `data` до того, как узнал, отказали ему или нет, и обязан быть готов получить
её вместо формы команды.

## Пример

```json
{
  "message": "--name must be a non-empty 1C identifier"
}
```
