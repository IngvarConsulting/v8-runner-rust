---
id: CTR.WIRE.TEST-DATA
version: 5
artifact: docs/schemas/command-data/test.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/cli_test.rs::test_all_full_json_runs_build_first_and_returns_report
  - src/use_cases/run_tests/helpers.rs::a_build_prerequisite_stopped_by_a_cancellation_is_an_interruption
  - src/use_cases/run_tests/helpers.rs::a_cancelled_run_is_classified_by_where_it_stopped
  - src/use_cases/run_tests.rs::run_tests_reports_cancelled_execution_before_first_safe_point
---

# `data` команды `test`

Самая крупная форма раннера: она несёт и разобранный отчёт прогона, и исход исполнения,
и пути удержанных артефактов. `error_kind` закрыт перечислением — по нему вызывающий
отличает упавшие тесты от неподнявшейся базы, не разбирая текст.

Живой проверки у формы пока нет: прогон требует настоящей платформы и установленного
YaXUnit. Форму держит сверка с типом, который её сериализует.

Отмена отвечает `status: cancelled`, записью о прерывании и ошибкой `cancelled` в
`execution.errors[]` с тем же текстом; `error_kind` ответа пуст, потому что отмена не вид
ошибки теста. Сборка-предпосылка, остановленная отменой, отвечает прерыванием, а не отказом
сборки: без ошибки `build_failed`, с записью фазы `command_boundary`, если сборку остановила
безопасная точка, или `provider_command`, если снят её исполнитель. Прогон, который отмена не
дала запустить, называется `command_boundary`, а не `run`: `run` — только прогон, снятый после
запуска.

Снятый прогон, конец которого не подтверждён, отвечает `error_kind:
enterprise_end_unconfirmed` со статусом `failed`, а не отменой и не истёкшим пределом.

**Что изменила версия 5.** В `error_kind` добавлено значение `enterprise_end_unconfirmed`.
Отмена по-прежнему пишет в `execution.errors[]` ошибку с кодом `cancelled` — тем же, что
отмена в конверте.

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
