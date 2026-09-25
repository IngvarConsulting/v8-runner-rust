---
id: CTR.WIRE.INFOBASE-CONFIGURATION-EXPORT-DATA
version: 4
artifact: docs/schemas/command-data/download.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it
---

# `data` команды `infobase configuration export`

Выгрузка пакета конфигурации из базы отчитывается не только результатом, но и выбором
исполнителя: квитанция `provider` называет выбранного, откуда взялся выбор (умолчание
матрицы или ключ `providers.*` с именем файла) и каждого пропущенного до него с причиной.
`selected: null` — выбор состоялся и никто не подошёл; отсутствие квитанции — выбор не
начинался, команда отказала раньше.

`target_state` говорит о состоянии базы после операции: выгрузка не меняет базу, и форма
это утверждает явно.

**Что изменила версия 4.** Фаза прерывания `execution.interruptions[].phase` стала закрытым
набором значений в `snake_case`, общим для всех форм с итогом исполнения; набор перечисляет
`$defs/ExecutionInterruptionPhase` схемы. Прежде фаза повторяла имя шага словами. Теперь
`provider command` стал `provider_command`, `publication` остался, а прерывание, замеченное на
остальных шагах, называется `command_boundary`. Не всякая остановка на безопасной точке даёт
запись: часть отвечает отказом без неё ([#308](https://github.com/IngvarConsulting/v8-runner-rust/issues/308)). Имена шагов в `steps[]` прежние.

## Пример

```json
{
  "mode": "preview",
  "provider_dispatched": false,
  "state": "working",
  "subject": {
    "kind": "main"
  },
  "provider": {
    "selected": null,
    "origin": {"kind": "default"},
    "skipped": [
      {"provider": "designer", "reason": "file infobase is not ready: 'build/ib/1Cv8.1CD' is missing or is not a file"},
      {"provider": "ibcmd", "reason": "file infobase is not ready: 'build/ib/1Cv8.1CD' is missing or is not a file"}
    ]
  },
  "artifact_kind": "cf",
  "output": "build/main.cf",
  "published": false,
  "target_state": "unchanged",
  "execution": {
    "status": "failed",
    "errors": [
      {
        "code": "environment_unavailable",
        "message": "environment unavailable: file infobase is not ready"
      }
    ]
  }
}
```
