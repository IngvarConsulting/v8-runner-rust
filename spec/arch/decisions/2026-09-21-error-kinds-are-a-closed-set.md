---
id: DEC.2026-09-21.ERROR-KINDS-ARE-A-CLOSED-SET
status: active
governs: product
realized: src/command_envelope.rs::every_error_kind_and_code_is_named_by_the_schema_and_by_a_table
supersedes: []
superseded-by: null
establishes: [CTR.WIRE.COMMAND-ENVELOPE]
changes: [CTR.WIRE.COMMAND-ENVELOPE]
---

# Роды ошибки — закрытое перечисление

**Решение.** `error.kind` и `error.code` в схеме конверта — закрытые перечисления, а не
строки. Сегодняшние рода остаются как есть: `validation`, `capability`, `environment`,
`workspace`, `platform`, `runtime`, `invalid_output`, `interruption`; добавляются
`non_fast_forward` и `no_memory` (имя решения, на сайте его нет). Рода отказа, которыми
сайт объясняет подбор команд — `subject` (предмет не тот, навсегда), `tool` (нет
инструмента в окружении), `target` (не для этой цели), `soon` (пока не умеет), — на
провод новыми родами не становятся: `tool` — это `environment` (инструмент не найден —
среда), `subject`, `target` и `soon` — `capability` (операция здесь не выполняется, и не
из-за входа), а различает их `error.code`: коды `subject`, `target` и `soon` заводятся в
закрытом наборе рядом с сегодняшним `capability_unavailable`, код выхода у них 2, как у
всякого `capability`. «Типизированный отказ» из расстановок — пара `kind` и `code`.

**Почему.** Свободная строка не удерживает перечень: новый род появляется без версии и
без проверки, а потребитель узнаёт о нём из журнала. Переименовывать сегодняшние
значения ради слов сайта значило бы ломать провод у Unica без выгоды.

**Отступление от #195.** Задача просила один закрытый набор `kind` из родов сайта и
`target_kind`; здесь рода подбора остаются категориями сайта, `target_kind` не заводится
— его предмет поглощён `capability` с `code`.

**Не затрагивает.** Коды выхода и `INV.CLI.EXIT-CODE-REFLECTS-THE-FAILURE-KIND`.

Источник: [`cli.html#refusals`](../../../docs/site/cli.html#refusals),
[`scenarios.html`](../../../docs/site/scenarios.html).
