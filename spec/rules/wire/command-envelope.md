---
id: CTR.WIRE.COMMAND-ENVELOPE
version: 3
artifact: docs/schemas/command-envelope.schema.json
check:
  - tests/contract_envelope.rs::a_successful_command_answers_in_the_pinned_envelope_form
  - src/command_envelope.rs::every_error_kind_and_code_is_named_by_the_schema_and_by_a_table
  - tests/cli_infobase_lock.rs::an_mcp_tool_on_a_held_base_is_refused_at_once
---

# Конверт ответа команды

Структурный ответ любой команды имеет одну оболочку, закреплённую схемой
`docs/schemas/command-envelope.schema.json`: `ok`, `command`, `duration_ms`, `data`,
`warnings`, `steps` и необязательная `error`. Отказ несёт `code`, `kind`, `message`, при
нужде `next` — следующий шаг структурой, — и два поля поколений у отказа
`non_fast_forward`. Список полей конверта, шага и отказа закрыт: новое поле у любого из
них валит проверку.

Рода и коды — закрытые перечисления, а не строки. Рода: `capability`, `environment`,
`workspace`, `invalid_output`, `interruption`, `validation`, `runtime`, `platform`,
`non_fast_forward`, `no_memory`. Коды подробнее рода: у `capability` их четыре —
`capability_unavailable`, `subject` (предмет не тот, навсегда), `target` (не для этой
цели) и `soon` (пока не умеет), и все отвечают кодом выхода 2. У `workspace` — занятость,
которая проходит, если повторить: `workspace_busy` (рабочий каталог держит другая команда)
и `infobase_busy` (файловую базу держит другая команда), с кодом выхода 3.

Схема порождается из типов: `UPDATE_ENVELOPE_SCHEMA=1 cargo test
generated_envelope_schema_is_current`. Руками её не правят — иначе закрытый набор
держится на внимательности.

**Заведено без производителя.** Рода `non_fast_forward` и `no_memory` и парные им коды
названы здесь целиком, чтобы набор не рос молча, но сегодня их не выдаёт никто: первого
приносит сравнение поколений, второго — память по базе. Тогда же у `non_fast_forward`
заполнятся `base_generation` и `local_generation`. Код `subject` таблица называет, и
сторож это проверяет, но ни один отказ пока этой причиной не отвечает. У шага `next`
сегодня заполняется только `command`: `source_set` и `keys` ждут отказов, которым есть
что в них положить.

**Что может назвать MCP.** Конверт один, словарь у транспортов разный: MCP сводит рода к
`validation`, `runtime` и `platform` (`DEC.2026-04-20.BUSINESS-FAILURES-ARE-NOT-TRANSPORT-FAULTS`),
поэтому ни одного кода возможности там не появляется — отказ по возможности приезжает как
`runtime_failure`. Так же приезжают занятый каталог и занятая база: MCP отказывает сразу,
а у занятой базы текст называет базу и команду, которая её держит. Шаг `next` от этого не
зависит и едет обоими транспортами: он про предмет, а не про провод.

**Что изменила версия 2.** Рода и коды стали перечислениями, у отказа появились `next` и
два поля поколений. Определения переименованы вслед за типами: `$defs/error` →
`$defs/EnvelopeError`, `$defs/step` → `$defs/StepResult`. Необязательные поля теперь
допускают `null` наравне с отсутствием — так порождает `schemars`, и так же устроены формы
данных команд; ни одно из них раннер `null`-ом не печатает. Состав шага описан целиком:
статусы, рода шага и артефакты стали закрытыми перечислениями вместо свободных строк.

**Что изменила версия 3.** Набор кодов вырос: `infobase_busy` — отказ на шаге
`infobase lock`, когда файловую базу держит другая команда. Рода не изменились. Коды,
которые приносят метка владельца базы, сравнение поколений и чужие изменения, входят в
эту же версию, когда появятся их производители.

**Два разных `code`.** Код отказа команды живёт в `error.code` этой формы. Код шага
исполнителя, который едет внутри `data.execution.errors[]`, — другой словарь и другая
запись; совпадение имён не делает их одним полем.

Предмет команды лежит в `data`, и этой формой он не описан: у каждой команды он свой.
Форму `data` закрепляет отдельный контракт на каждую форму — перечень объявлен в
`docs/schemas/command-data/index.json` и разобран по записям `CTR.WIRE.*-DATA`. Поле
`next` живёт здесь, а не в `CTR.WIRE.REFUSAL-DATA`: та форма описывает отказ до запуска
команды, а отказать со следующим шагом может и команда, уже начавшая работу, — у неё в
`data` лежит её собственный предмет.

## Пример

```json
{
  "ok": false,
  "command": "launch",
  "duration_ms": 4,
  "data": {},
  "warnings": [],
  "steps": [
    {
      "name": "resolve-target",
      "ok": false,
      "status": "failed",
      "kind": "resolve_target",
      "duration_ms": 3,
      "message": "standalone target is opened by its web address"
    }
  ],
  "error": {
    "code": "target",
    "kind": "capability",
    "message": "a standalone server is opened by its web address: use `launch web` with infobase.web.url; a client is not launched against the gate",
    "next": {
      "command": "launch web"
    }
  }
}
```
