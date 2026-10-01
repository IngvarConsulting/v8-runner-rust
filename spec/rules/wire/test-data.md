---
id: CTR.WIRE.TEST-DATA
version: 3
artifact: docs/schemas/command-data/test.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/cli_test.rs::test_all_full_json_runs_build_first_and_returns_report
  - src/use_cases/run_tests/helpers.rs::a_build_prerequisite_stopped_by_a_cancellation_is_an_interruption
  - src/use_cases/run_tests/helpers.rs::a_cancelled_run_is_classified_by_where_it_stopped
---

# `data` команды `test`

Самая крупная форма раннера: она несёт и разобранный отчёт прогона, и исход исполнения,
и пути удержанных артефактов. `error_kind` закрыт перечислением — по нему вызывающий
отличает упавшие тесты от неподнявшейся базы, не разбирая текст.

Живой проверки у формы пока нет: прогон требует настоящей платформы и установленного
YaXUnit. Форму держит сверка с типом, который её сериализует.

**Что изменила версия 3.** Значение `export_or_publication` ушло из общего набора фаз;
`test` его не давал. Сборка-предпосылка, остановленная отменой, отвечает прерыванием, а не
отказом сборки: `status: cancelled` без ошибки `build_failed` и запись с фазой
`command_boundary`, если сборку остановила безопасная точка, или `provider_command`, если снят
её исполнитель. Прогон, который отмена не дала запустить, называется `command_boundary`, а не
`run`: `run` — только прогон, снятый после запуска.

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
