---
id: CTR.WIRE.COMMAND-ENVELOPE
version: 6
artifact: docs/schemas/command-envelope.schema.json
check:
  - tests/contract_envelope.rs::a_successful_command_answers_in_the_pinned_envelope_form
  - src/command_envelope.rs::every_error_kind_and_code_is_named_by_the_schema_and_by_a_table
  - src/command_envelope.rs::every_warning_code_is_named_by_the_schema_and_by_a_producer
  - tests/cli_infobase_owner.rs::a_preview_on_a_base_of_another_copy_gives_the_warning_code_and_writes_nothing
  - tests/cli_infobase_owner.rs::an_own_base_and_a_base_without_a_marker_give_no_warning_code
  - tests/cli_infobase_lock.rs::an_mcp_tool_on_a_held_base_is_refused_at_once
  - tests/cli_infobase_owner.rs::a_write_on_a_base_of_another_copy_runs_with_a_warning_and_names_the_owner
  - tests/cli_platform_release.rs::every_platform_command_refuses_without_the_release_of_the_sources_format
  - tests/cli_apply_sessions.rs::a_file_base_held_by_a_session_is_refused_with_the_session_list
  - src/command_envelope.rs::the_environment_layer_set_is_closed_and_named_by_the_schema
  - tests/cli_dump_agent.rs::a_rejected_password_is_an_environment_refusal_even_though_the_port_answers
---

# Конверт ответа команды

Структурный ответ любой команды имеет одну оболочку, закреплённую схемой
`docs/schemas/command-envelope.schema.json`: `ok`, `command`, `duration_ms`, `data`,
`warnings`, необязательная `warning_details`, `steps` и необязательная `error`. Отказ несёт `code`, `kind`, `message`, при
нужде `next` — следующий шаг структурой, — два поля поколений у отказа
`non_fast_forward`, поле `platform_release` у отказа `platform_release_missing`, поле `sessions` у отказов `infobase_sessions` и `infobase_designer_busy` и поле `layer` у отказа `environment_unavailable`, когда слой, который отказал, определён. Список полей конверта, шага и отказа закрыт: новое поле у любого из
них валит проверку.

Рода и коды — закрытые перечисления, а не строки. Рода: `capability`, `environment`,
`workspace`, `invalid_output`, `interruption`, `validation`, `runtime`, `platform`,
`non_fast_forward`, `untracked_base`. Коды подробнее рода: у `capability` их четыре —
`capability_unavailable`, `subject` (предмет не тот, навсегда), `target` (не для этой
цели) и `soon` (пока не умеет), и все отвечают кодом выхода 2. Коды рода `environment` тоже отвечают
кодом выхода 2: `environment_unavailable` (утилиты нет или она не та) и
`platform_release_missing` (при `tools.platform.version: sources` нет выпуска, который пишет
формат исходников; `INV.PLATFORM.A-MISSING-RELEASE-OF-THE-SOURCES-FORMAT-IS-REFUSED`). Поле
`layer` отказа `environment_unavailable` называет слой из закрытого набора `infobase_user`,
`agent_endpoint`, `agent_start`, `platform`; без поля слой не определён
(`INV.WIRE.AN-ENVIRONMENT-REFUSAL-NAMES-ITS-LAYER`). У `platform_release_missing`
поле `platform_release` несёт `source_format` — версию формата, файл и выпуск, как в ответе
`platforms`, — и `installations`, опись установок, по которой решён отказ. У `workspace` — чужая работа
с тем же местом, с кодом выхода 3: занятость, которая проходит, если повторить, —
`workspace_busy` (рабочий каталог держит другая команда), `infobase_busy` (файловую базу
держит другая команда), `infobase_sessions` (обновлению конфигурации базы данных нужен
монопольный доступ, а базу держат чужие сеансы) и `infobase_designer_busy` (файловую базу
держит чужой Конфигуратор). Поле `sessions` у отказов «базу держат» называет базу, сеансы, можно ли их
завершить, блокировку начала сеансов, источник признака (`evidence`) и, если признак — список
сервера, отказ исполнителя (`executor_error`)
(`INV.CLI.APPLY-SESSIONS-GOVERNS-FOREIGN-SESSIONS`). База другой рабочей копии отказом не служит: команда идёт, а
`warnings` несёт предупреждение
(`INV.USE-CASES.A-WRITE-ON-A-BASE-OF-ANOTHER-COPY-RUNS-WITH-A-WARNING`), и у него есть код. У рода `platform`, код
выхода 4, — `platform_failure` и `agent_command_abandoned`: второй сигнал бросил ожидание
команды чужого агента, и она, возможно, ещё идёт
(`INV.USE-CASES.AN-ATTACHED-AGENT-ANSWERS-CANCELLED-AFTER-ITS-COMMAND-ENDS`).

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

**Что изменила версия 5.** Код и род отказа `no_memory` называются `untracked_base`: база этой рабочей копией не отслеживается — нет её памяти о базе, как у ветки без слежения в git. Условия отказа и выходы те же. Набор кодов вырос на три кода удержания живого агента: `agent_limit`, `live_agent_off` и `live_agent_job`; рода прежние. У рода `environment` появился и код `platform_release_missing`, а у отказа — необязательное поле `platform_release`, которое заполняет только он. В эту же версию вошёл код `source_set_ignored_by_git` рода `validation`, код выхода 2: правило игнорирования git прячет файл набора, изменения которого берутся у git ([правило](../use-cases/a-set-hidden-by-a-git-ignore-rule-is-refused.md)). Так же, родом `validation` с кодом выхода 2, в неё вошёл код `work_path_on_another_volume`: каталог набора и `workPath` на разных томах, и `pull` не начинается ([правило](../use-cases/the-dump-stage-lies-in-the-work-path-on-the-set-volume.md)). Рода `workspace` прибавили коды `infobase_sessions` и `infobase_designer_busy`, а у отказа — необязательное поле `sessions`, которое заполняют только они. Роду `platform` прибавился код `agent_command_abandoned`, код выхода 4.

**Что изменила версия 6.** У конверта новое необязательное поле `warning_details` —
предупреждения с машинным кодом рядом с прежними строками `warnings`, которые не изменились.
Первый код — `infobase_of_another_copy`. Поле появляется, только когда такое предупреждение
есть, поэтому прежние ответы не изменились. Здесь же у отказа появилось необязательное поле
`layer`: его заполняет только `environment_unavailable`, когда слой, который отказал, определён, — `infobase_user`,
`agent_endpoint`, `agent_start` или `platform`
(`INV.WIRE.AN-ENVIRONMENT-REFUSAL-NAMES-ITS-LAYER`). Рода, коды и тексты отказов прежние:
поле только добавлено, и потребитель, который его не знает, читает конверт как раньше. Тем же
полем слой едет у MCP — в отказе и в конверте ответа.

**Что изменила версия 4.** Коды рода `workspace` — `workspace_busy` и `infobase_busy`: запись в
базу другой рабочей копии не отказывает, а предупреждает в `warnings`.

**Предупреждения с кодом.** `warnings` — строки для людей, и форма их не меняется.
Предупреждение, которое вызывающий узнаёт без разбора текста, едет ещё и в `warning_details`:
объект с `code`, `message` — та же строка, что стоит в `warnings`, — и полями своего кода.
Порядок `warning_details` — порядок их строк в `warnings`; поля нет, когда таких
предупреждений нет. Набор кодов закрыт, объект каждого кода закрыт, как у отказа. Код один —
`infobase_of_another_copy`: команда записи идёт на файловой базе другой живой рабочей копии.
Его поля: `name` — имя базы в местном слое (`null` у строки соединения), `address` — адрес без
учётных данных, тот же, что `address` у `status`, и `owners` — живые копии-владельцы, кроме
этой, той же записью, что `holders.owners` у `status` (`project`, `host`, `since`, `this_copy`,
`this_machine`). Код приходит у прогона и у превью (`--dry-run`) командной строки и у
инструмента MCP; превью у MCP нет, поэтому там — у прогона.

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
