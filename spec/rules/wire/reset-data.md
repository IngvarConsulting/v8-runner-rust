---
id: CTR.WIRE.RESET-DATA
version: 1
artifact: docs/schemas/command-data/reset.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it
  - tests/cli_reset.rs::reset_discards_a_push_without_apply_and_the_next_push_loads_it_again
  - tests/cli_reset.rs::reset_with_nothing_unapplied_rolls_nothing_back
  - tests/cli_reset.rs::a_failed_rollback_answers_a_platform_failure_and_the_next_push_loads_everything
  - tests/cli_reset.rs::reset_creates_no_memory_of_the_base
  - tests/cli_reset.rs::reset_into_a_base_that_moved_keeps_the_record_and_the_next_push_is_refused
---

# `data` команды `reset`

Цель у команды одна: набор `source_set` с назначением `purpose` (как ключ `type` проекта) —
основная конфигурация или расширение. Исход `outcome`: `discarded` — непринятое отброшено;
`nothing_to_discard` — непринятого не было, отката нет и ничего не записано; `planned` у
превью; `failed` — команда отказала, и `error` конверта называет почему.

`hash_memory` говорит, что стало с хеш-памятью набора перед откатом: `replaced` — своя
хеш-память заменена пустой; `absent` — своей непустой хеш-памяти нет, ничего не записано.
Поля нет — команда до памяти не дошла: превью, нечего отбрасывать или отказ раньше.
`generation` говорит, что стало с записью журнала поколений набора после отката, теми же
значениями, что у `apply` (`CTR.WIRE.APPLY-DATA`): `recorded` — поколение до отката совпало
с записью, и после него записан ответ инструмента записи, признак «не применено» снят;
`kept` — база ушла от записи до отката или запись сделана перед неудачной загрузкой: запись
не тронута, и следующая отправка это назовёт; `erased` — ответа нет, запись стёрта;
`unerased` — стереть не удалось; `unchecked` — своей записи нет.
Поля нет — отката не было, или отмена прервала чтение поколения после него (сообщение
называет и то и другое).

Отказ до выбора цели — неизвестный, внешний набор или проект без набора основной
конфигурации — формы команды не несёт: исполнителя ещё нет.

## Пример

```json
{
  "provider": {"selected": "designer", "origin": {"kind": "default"}},
  "ok": true,
  "provider_dispatched": true,
  "source_set": "main",
  "purpose": "CONFIGURATION",
  "outcome": "discarded",
  "hash_memory": "replaced",
  "generation": "recorded",
  "message": "rolled the main configuration of source-set 'main' back to the database configuration",
  "duration_ms": 5310
}
```
