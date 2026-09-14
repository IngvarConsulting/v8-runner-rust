---
id: CTR.WIRE.VERSION-DATA
status: active
governs: product
version: 1
decision: DEC.2026-09-14.EVERY-COMMAND-PINS-THE-FORM-OF-ITS-DATA
artifact: docs/schemas/command-data/version.schema.json
producer: src/app.rs
consumers: [cli, unica]
check: [src/command_data.rs::generated_command_data_schemas_are_current, tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it]
scope: [wire, cli]
---

# `data` команды `version`

Имя и версия самого раннера — всё, что команда сообщает. Предмета в базе у неё нет,
платформа не запускается, поэтому форма не меняется от конфигурации проекта и годится
как проба живости: если вызывающий получил её целиком, канал ответов работает.

## Пример

```json
{
  "name": "v8-runner",
  "version": "0.9.0"
}
```
