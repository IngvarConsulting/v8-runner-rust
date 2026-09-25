---
id: CTR.WIRE.INFOBASE-DUMP-DATA
version: 4
artifact: docs/schemas/command-data/infobase-dump.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it
---

# `data` команды `infobase dump`

Снимок базы целиком (`.dt`) отчитывается той же логикой выбора провайдера, что и выгрузка
пакета, но своим предметом: `subject.kind` здесь — вся база, а не конфигурация внутри неё.
Формы разведены, потому что состав полей у них расходится и будет расходиться дальше.

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
  "subject": {
    "kind": "infobase"
  },
  "provider": {
    "selected": null,
    "origin": {"kind": "default"},
    "skipped": [{"provider": "designer", "reason": "file infobase is not ready: 'build/ib/1Cv8.1CD' is missing or is not a file"}]
  },
  "artifact_kind": "dt",
  "output": "build/main.dt",
  "published": false,
  "target_state": "unchanged",
  "execution": {
    "status": "failed",
    "errors": [
      {
        "code": "environment_unavailable",
        "message": "environment unavailable: no provider is ready"
      }
    ]
  }
}
```
