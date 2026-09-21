---
id: DEC.2026-09-21.INFOBASES-ARE-A-NAMED-MAP-WITH-ORIGIN-AS-THE-DEFAULT
status: active
governs: product
realized: tests/cli_infobases.rs::origin_is_selected_without_a_flag
supersedes: [DEC.2026-04-21.INFOBASE-SECTION-OWNS-CONNECTION-AND-CREDENTIALS]
superseded-by: null
establishes: [INV.CLI.A-COMMAND-WITHOUT-ORIGIN-NAMES-THE-MISSING-STEP, INV.CONFIG.TOP-LEVEL-CONNECTION-IS-NOT-ACCEPTED, INV.CONFIG.DBMS-IS-REJECTED-FOR-A-FILE-BASE, INV.CONFIG.AN-INFOBASE-NAME-IS-A-PLAIN-IDENTIFIER, CTR.CONFIG.V8PROJECT-SCHEMA]
changes: [CTR.CONFIG.V8PROJECT-SCHEMA, INV.CONFIG.OVERLAY-KEEPS-ITS-SCOPE]
---

# Базы объявляются по именам, умолчание — `origin`

**Решение.** У проекта сколько угодно именованных баз: карта `infobases` в местном слое
`v8project.local.yaml`, умолчание зовётся `origin`. Сверх сайта: в проектном файле карта
не принимается — к какой базе подключён каталог, знает только эта машина, а сценарий
сборки называет базу ключом; прежний `infobase:` там доживает цикл синонимом
(`DEC.2026-09-21.OLD-NAMES-ARE-HIDDEN-SYNONYMS-FOR-ONE-CYCLE`). Секция базы владеет
всем, что о ней известно: `connection`, `user`, `password`, `cluster`, `standalone`,
`web`, `dbms`; ключей верхнего уровня для этого нет. Любая команда, которой нужна база,
принимает `--infobase` с именем или строкой соединения целиком; без ключа идёт в
`origin`. Если `origin` не объявлен, команда не гадает, а отказывает и называет шаг:
`init --infobase …` или `--infobase <имя>`. Отдельной команды `remote` нет: базы
объявляют в местном слое, адреса видны в `status --all`, действия с базой лежат под
`infobase`. Сверх сайта: имя базы — идентификатор из латинских букв, цифр, `-` и `_`,
первый знак — буква или цифра, не длиннее 64 знаков; оно же сегмент пути под `workPath`,
и другого имени схема не принимает. База, названная строкой соединения, не объявлена:
памяти у неё нет (`DEC.2026-09-21.MEMORY-IS-KEPT-PER-INFOBASE`).

**Почему.** Одна секция `infobase` заставляла держать по конфигу на базу и не давала
одной командой отправить в `test`, не задев `prod`. Имя базы заодно становится ключом
памяти о ней (`DEC.2026-09-21.MEMORY-IS-KEPT-PER-INFOBASE`).

**Цена.** Новая форма схемы конфигурации; прежний ключ `infobase:` — скрытый синоним
`infobases.origin` на один цикл.

**Заменяет при реализации.**
`DEC.2026-04-21.INFOBASE-SECTION-OWNS-CONNECTION-AND-CREDENTIALS`. Его правила
`INV.CONFIG.TOP-LEVEL-CONNECTION-IS-NOT-ACCEPTED` и
`INV.CONFIG.DBMS-IS-REJECTED-FOR-A-FILE-BASE` остаются верными и переходят сюда;
`INV.CONFIG.CREDENTIALS-STAY-IN-THE-OVERLAY` не затронуто.

Источник: [`cli.html#map`](../../../docs/site/cli.html#map),
[`index.html`](../../../docs/site/index.html),
[`sources.html#memory`](../../../docs/site/sources.html#memory).
