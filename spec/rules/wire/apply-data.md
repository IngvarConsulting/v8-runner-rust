---
id: CTR.WIRE.APPLY-DATA
version: 1
artifact: docs/schemas/command-data/apply.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it
  - tests/cli_apply.rs::a_push_without_apply_loads_and_apply_applies_it
  - tests/cli_apply.rs::an_apply_into_a_base_that_moved_keeps_the_record
---

# `data` команды `apply`

Применение идёт по наборам в порядке проекта — основная конфигурация, расширения, внешние
наборы, — а без набора за ними идёт расширение-инструмент клиентского MCP (`tool:<имя>`).
Шаг называет набор, его назначение (`purpose`, как ключ `type` проекта) и исход `outcome`:
`applied`, `planned` у превью, `skipped` у внешнего набора и у расширения, которого нет в
базе, `failed` у шага, на котором команда остановилась, `not_run` у шагов после него.
Отдельного `ok` у шага нет: удача шага — его исход.

Поле `generation` говорит, что стало с записью журнала поколений набора: `recorded` — поколение
до применения совпало с записью, и после него записан ответ того же инструмента; `kept` — база
ушла от записи, запись не тронута; `erased` — инструмент записи не ответил, запись стёрта;
`unchecked` — записи нет; `unerased` — инструмент записи не ответил, а стереть запись не
удалось, и сообщение шага называет причину. У шага, который не применялся, и у расширения-инструмента поля нет.

## Пример

```json
{
  "provider": {"selected": "designer", "origin": {"kind": "default"}},
  "ok": true,
  "provider_dispatched": true,
  "duration_ms": 2140,
  "steps": [
    {
      "source_set": "main",
      "purpose": "CONFIGURATION",
      "outcome": "applied",
      "message": "applied the main configuration to the database configuration",
      "duration_ms": 2100,
      "generation": "recorded"
    },
    {
      "source_set": "epf",
      "purpose": "EXTERNAL_DATA_PROCESSORS",
      "outcome": "skipped",
      "message": "external files are not loaded into the infobase",
      "duration_ms": 0
    }
  ]
}
```
