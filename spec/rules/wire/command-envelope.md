---
id: CTR.WIRE.COMMAND-ENVELOPE
version: 5
artifact: docs/schemas/command-envelope.schema.json
check:
  - tests/contract_envelope.rs::a_successful_command_answers_in_the_pinned_envelope_form
  - src/command_envelope.rs::every_error_kind_and_code_is_named_by_the_schema_and_by_a_table
  - tests/cli_infobase_lock.rs::an_mcp_tool_on_a_held_base_is_refused_at_once
  - tests/cli_infobase_owner.rs::a_write_on_a_base_of_another_copy_runs_with_a_warning_and_names_the_owner
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
`non_fast_forward`, `untracked_base`. Коды подробнее рода: у `capability` их четыре —
`capability_unavailable`, `subject` (предмет не тот, навсегда), `target` (не для этой
цели) и `soon` (пока не умеет), и все отвечают кодом выхода 2. У `workspace` — чужая работа
с тем же местом, с кодом выхода 3: занятость, которая проходит, если повторить, —
`workspace_busy` (рабочий каталог держит другая команда) и `infobase_busy` (файловую базу
держит другая команда). База другой рабочей копии отказом не служит: команда идёт, а
`warnings` несёт предупреждение
(`INV.USE-CASES.A-WRITE-ON-A-BASE-OF-ANOTHER-COPY-RUNS-WITH-A-WARNING`).

Схема порождается из типов: `UPDATE_ENVELOPE_SCHEMA=1 cargo test
generated_envelope_schema_is_current`. Руками её не правят — иначе закрытый набор
держится на внимательности.

**Отказы обмена с базой.** Род и код `untracked_base` — `push` в базу, о которой у рабочей копии нет
памяти; `non_fast_forward` — `push` в базу, ушедшую вперёд записанного поколения, и только у
него заполнены поля `base_generation` (поколение базы сейчас) и `local_generation` (записанное
после прошлого обмена). Тем же кодом, но без этих полей отвечают отказы, в которых уход вперёд
не доказан и не опровергнут: `apply` после неудачной загрузки, когда поколения нет, и `push`,
когда поколение записи другого инструмента спросить не удалось
(`INV.USE-CASES.A-PUSH-INTO-A-BASE-THAT-MOVED-AHEAD-IS-REFUSED`) — текст называет причину. Код выхода у этих отказов — 3. Их `next` — `pull` с `source_set`, а
когда выгрузку предлагать нельзя, — `push` с `keys` `{"--force": ""}`: ключ без значения
несёт пустую строку. Коды `subject` и `soon` таблица называет, и сторож это проверяет, но ни
один отказ пока этими причинами не отвечает.

**Что может назвать MCP.** Конверт один, словарь у транспортов разный: MCP сводит рода к
`validation`, `runtime` и `platform` (`DEC.2026-04-20.BUSINESS-FAILURES-ARE-NOT-TRANSPORT-FAULTS`),
поэтому ни одного кода возможности там не появляется — отказ по возможности приезжает как
`runtime_failure`. Так же приезжают занятый каталог и занятая база: MCP отказывает сразу, у
занятой базы текст называет базу и команду, которая её держит. Шаг `next` от этого не
зависит и едет обоими транспортами: он про предмет, а не про провод.

**Что изменила версия 2.** Рода и коды стали перечислениями, у отказа появились `next` и
два поля поколений. Определения переименованы вслед за типами: `$defs/error` →
`$defs/EnvelopeError`, `$defs/step` → `$defs/StepResult`. Необязательные поля теперь
допускают `null` наравне с отсутствием — так порождает `schemars`, и так же устроены формы
данных команд; ни одно из них раннер `null`-ом не печатает. Состав шага описан целиком:
статусы, рода шага и артефакты стали закрытыми перечислениями вместо свободных строк.

**Что изменила версия 3.** Набор кодов вырос: `infobase_busy` — отказ на шаге
`infobase lock`, когда файловую базу держит другая команда, и `infobase_held` — отказ на шаге
`infobase owner`, когда её держит другая рабочая копия (убран в версии 4); его `next` — своя чистая база,
`infobase create`. Рода не изменились. В эту же версию вошли производители кодов
`non_fast_forward` и `untracked_base` и шаг `next` с `source_set` и `keys`; код чужих изменений
войдёт в неё же, когда появится его производитель.

**Отказы удержания живого агента.** `agent hold` отказывает тремя кодами
(`INV.CLI.AN-AGENT-HOLD-OVER-THE-LIMIT-IS-REFUSED`): `agent_limit` рода `environment`, код
выхода 2, — живых агентов учётной записи уже `tools.designer_agent.live-max`, и все они
удерживаются; `live_agent_off` рода `capability`, код выхода 2, — живой агент
выключен `tools.designer_agent.live: false`; `live_agent_job` рода `environment`, код выхода
2, — раннер в Job, из которого сторож не может выйти. Платформу ни один из них не запускает.

**Что изменила версия 5.** Код и род отказа `no_memory` называются `untracked_base`: база этой рабочей копией не отслеживается — нет её памяти о базе, как у ветки без слежения в git. Условия отказа и выходы те же. Набор кодов вырос на три кода удержания живого агента: `agent_limit`, `live_agent_off` и `live_agent_job`; рода прежние. В эту же версию вошёл код `source_set_ignored_by_git` рода `validation`, код выхода 2: правило игнорирования git прячет файл набора, изменения которого берутся у git ([правило](../use-cases/a-set-hidden-by-a-git-ignore-rule-is-refused.md)).

**Что изменила версия 4.** Коды рода `workspace` — `workspace_busy` и `infobase_busy`: запись в
базу другой рабочей копии не отказывает, а предупреждает в `warnings`.

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
      "message": "a standalone server is not opened by the thick client"
    }
  ],
  "error": {
    "code": "target",
    "kind": "capability",
    "message": "the thick client and the ordinary application are not launched against a standalone server: it is opened by the thin client, by the Designer or in a browser",
    "next": {
      "command": "launch thin"
    }
  }
}
```
