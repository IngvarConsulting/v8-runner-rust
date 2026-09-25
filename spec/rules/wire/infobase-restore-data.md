---
id: CTR.WIRE.INFOBASE-RESTORE-DATA
version: 4
artifact: docs/schemas/command-data/infobase-restore.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it
---

# `data` команды `infobase restore`

Подъём базы из снимка — единственная команда этой тройки, которая базу меняет, и форма
говорит об этом двумя разными полями: `target_mode` — что просили сделать с целью
(создать или заместить), `restored` — сделано ли. `target_state` остаётся `unchanged`,
пока платформа не отработала: превью цель не трогает.

**Что изменила версия 4.** Фаза прерывания `execution.interruptions[].phase` стала закрытым
набором значений в `snake_case`, общим для всех форм с итогом исполнения; набор перечисляет
`$defs/ExecutionInterruptionPhase` схемы. Прежде фаза повторяла имя шага словами. Теперь
`provider command` стал `provider_command`, а прерывание, замеченное на остальных шагах,
называется `command_boundary`. Не всякая остановка на безопасной точке даёт запись: часть
отвечает отказом без неё ([#308](https://github.com/IngvarConsulting/v8-runner-rust/issues/308)). Имена шагов в `steps[]` прежние.

## Пример

```json
{
  "mode": "preview",
  "provider_dispatched": false,
  "subject": {
    "kind": "infobase"
  },
  "provider": {
    "selected": null,
    "origin": {"kind": "default"},
    "skipped": []
  },
  "artifact_kind": "dt",
  "input": "build/main.dt",
  "target_mode": "replace",
  "restored": false,
  "target_state": "unchanged",
  "execution": {
    "status": "failed",
    "errors": [
      {
        "code": "invalid_argument",
        "message": "infobase restore requires exactly one of --create or --replace"
      }
    ]
  }
}
```
