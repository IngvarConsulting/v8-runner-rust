---
id: CTR.WIRE.PROGRESS-EVENTS
version: 1
artifact: docs/schemas/progress-event.schema.json
check:
  - src/output/events.rs::generated_progress_event_schema_is_current
  - src/output/events.rs::every_progress_event_passes_the_schema_and_nothing_else_does
  - src/output/events.rs::a_stream_counts_its_steps_cancels_once_and_falls_silent_after_the_envelope
  - tests/cli_progress_events.rs::events_stream_the_agent_progress_and_the_envelope_comes_last_on_one_line
  - tests/cli_progress_events.rs::events_reach_stdout_while_the_agent_still_works
  - tests/cli_progress_events.rs::events_without_json_message_are_refused_at_start
  - tests/cli_progress_events.rs::events_are_refused_by_the_mcp_server
  - tests/cli_progress_events.rs::events_are_refused_next_to_publication_stdio
---

# События хода команды в stdout

`--events -` вместе с `--json-message` превращает stdout в NDJSON: события хода, каждое
отдельной строкой, сброшенной сразу, и последней строкой — итоговый конверт
`CTR.WIRE.COMMAND-ENVELOPE`, тоже одной строкой. После конверта в stdout ничего не пишется.
Поток без конверта значит, что раннер упал, не ответив. Без ключа stdout прежний: один
конверт с отступами.

Ключ отвергается при старте, до работы, текстом в stderr и с пустым stdout: без
`--json-message`, рядом с `--publication-stdio` и у `mcp serve` — там stdout занят текстом,
кадрами публикации и протоколом MCP. Единственное значение ключа — `-`.

Каждое событие несёт `version`, `event`, `at` (RFC 3339 в UTC) и `command` — имя команды, как
его называет конверт. Видов событий закрытый перечень, форма закрыта схемой
`docs/schemas/progress-event.schema.json`:

- `step` — команда начала шаг. `phase` — из закрытого перечня фаз: `preview`, `session`,
  `wait`, `inspect`, `forecast`, `convert`, `dump`, `load`, `update_db`, `reset`,
  `extensions`, `check`, `package`, `infobase`, `test`, `launch`, `publish`. С `steps`
  конверта фаза не сверяется. `set` — набор исходников шага или `null`. `step_index` растёт
  на единицу с каждым шагом; `step_count` — сколько шагов известно сейчас, не меньше
  `step_index`, и может расти.
- `agent` — промежуточное сообщение агента Конфигуратора: `agent_type` — `progress`, `log`
  или `loading_issue`, `text` — его текст с маской учётных данных
  (`INV.USE-CASES.A-LONG-AGENT-COMMAND-SHOWS-ITS-PROGRESS`).
- `cancel` — первый сигнал отмены, `reason` — `cancelled` или `timed_out`. Пишется
  единожды; строки агента после него ещё приходят, пока команда не ответит. У команды
  командной строки срока нет, и `timed_out` она пока не пишет.

Запись в stdout у потока одна и под замком: событие из потока сигнала не врезается в
конверт.

## Пример

```json
{"version":1,"event":"step","at":"2026-11-11T15:32:33.131Z","command":"pull","phase":"dump","set":"main","step_index":2,"step_count":2}
```

```json
{"version":1,"event":"agent","at":"2026-11-11T15:32:33.134Z","command":"pull","agent_type":"progress","text":"Выгрузка конфигурации 50%"}
```

```json
{"version":1,"event":"cancel","at":"2026-11-11T15:32:34.002Z","command":"pull","reason":"cancelled"}
```
