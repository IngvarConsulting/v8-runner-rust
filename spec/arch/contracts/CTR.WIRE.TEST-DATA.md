---
id: CTR.WIRE.TEST-DATA
status: active
governs: product
version: 1
decision: DEC.2026-09-14.EVERY-COMMAND-PINS-THE-FORM-OF-ITS-DATA
artifact: docs/schemas/command-data/test.schema.json
producer: src/command_envelope.rs
consumers: [cli, mcp, unica]
check: [src/command_data.rs::generated_command_data_schemas_are_current, tests/cli_test.rs::test_all_full_json_runs_build_first_and_returns_report]
scope: [wire, cli, mcp]
---

# `data` команды `test`

Самая крупная форма раннера: она несёт и разобранный отчёт прогона, и исход исполнения,
и пути удержанных артефактов. `error_kind` закрыт перечислением — по нему вызывающий
отличает упавшие тесты от неподнявшейся базы, не разбирая текст.

Живой проверки у формы пока нет: прогон требует настоящей платформы и установленного
YaXUnit. Форму держит сверка с типом, который её сериализует.

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
