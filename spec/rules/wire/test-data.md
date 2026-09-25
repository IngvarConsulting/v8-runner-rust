---
id: CTR.WIRE.TEST-DATA
version: 2
artifact: docs/schemas/command-data/test.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/cli_test.rs::test_all_full_json_runs_build_first_and_returns_report
---

# `data` команды `test`

Самая крупная форма раннера: она несёт и разобранный отчёт прогона, и исход исполнения,
и пути удержанных артефактов. `error_kind` закрыт перечислением — по нему вызывающий
отличает упавшие тесты от неподнявшейся базы, не разбирая текст.

Живой проверки у формы пока нет: прогон требует настоящей платформы и установленного
YaXUnit. Форму держит сверка с типом, который её сериализует.

**Что изменила версия 2.** Фаза прерывания `execution.interruptions[].phase` стала закрытым
набором значений в `snake_case`, общим для всех форм с итогом исполнения; набор перечисляет
`$defs/ExecutionInterruptionPhase` схемы. Значения `test` — `command_boundary` и `run` —
прежние.

## Пример

```json
{
  "ok": true,
  "target": "all",
  "mode": "compact",
  "diagnostics": [],
  "report": {
    "summary": {
      "total": 12,
      "passed": 12,
      "failed": 0,
      "skipped": 0,
      "errors": 0
    },
    "suites": [
      {
        "name": "ОбщийМодуль.ДемоТесты",
        "duration_ms": 184,
        "cases": [
          {
            "name": "ТестСложения",
            "status": "PASSED",
            "duration_ms": 12
          }
        ]
      }
    ],
    "extracted_errors": []
  },
  "execution": {
    "status": "succeeded",
    "diagnostics": [],
    "errors": []
  }
}
```
