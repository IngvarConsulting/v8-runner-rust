---
id: CTR.WIRE.VERSION-DATA
version: 1
artifact: docs/schemas/command-data/version.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it
---

# `data` команды `version`

Имя и версия самого раннера — всё, что команда сообщает. Предмета в базе у неё нет,
платформа не запускается, поэтому форма не меняется от конфигурации проекта и годится
как проба живости: если вызывающий получил её целиком, канал ответов работает.

## Пример

```json
{
  "name": "v8-runner",
  "version": "0.11.0"
}
```
