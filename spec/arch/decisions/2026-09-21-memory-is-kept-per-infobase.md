---
id: DEC.2026-09-21.MEMORY-IS-KEPT-PER-INFOBASE
status: planned
governs: product
realized: null
supersedes: []
superseded-by: null
establishes: [INV.USE-CASES.FOREIGN-MEMORY-IS-NOT-USED, INV.USE-CASES.A-BASE-NAMED-BY-A-CONNECTION-STRING-LEAVES-NO-MEMORY]
---

# Память о прошлом разе хранится по базе

**Решение.** Три памяти раннера — хеши исходников, поколение базы и файл версий
`ConfigDumpInfo.xml` — описывают отношение «один каталог ↔ одна база», поэтому лежат под
`workPath/infobases/<имя базы>/`: `generation.json`, `hashes/` по наборам,
`dump-info/<набор>/ConfigDumpInfo.xml`. Ключ записи о поколении — предмет (конфигурация
или расширение), инструмент и операция. При первом обращении память запоминает, к какой
базе относится; если под тем же именем оказалась другая база, память объявляется чужой и
не используется. Имя базы входит в путь как есть — его форму ограничивает схема
(`INV.CONFIG.AN-INFOBASE-NAME-IS-A-PLAIN-IDENTIFIER`). База, названная в `--infobase`
строкой соединения, памяти не имеет: обмен с ней всегда первый
(`DEC.2026-09-21.THE-GENERATION-GUARDS-EVERY-EXCHANGE`), память о ней не пишется, а
`status` отвечает, что памяти нет; кому нужна память, объявляет базу по имени. Отправка
в `test` не сбивает память о `prod`. Под базой лежит только состояние обмена с ней —
контекст `designer-<набор>`; кеш экспорта EDT и контекст `edt-<набор>` от базы не
зависят и остаются общими.

**Почему.** Сегодня память ключуется именем набора и лежит в одном `workPath`:
подключите вторую базу — раннер молча возьмёт память первой, доложит «изменений нет» и
ничего не сделает. Ошибка не в отказе, а в бездействии.

**Не затрагивает.** `DEC.2026-04-20.WORKPATH-IS-THE-ONLY-STATE-ROOT` — корень состояния
один, меняется раскладка под ним; `DEC.2026-04-20.EDT-EXPORT-KEEPS-ITS-OWN-CHANGE-STATE`
— две ступени EDT остаются двумя.

Источник: [`sources.html#memory`](../../../docs/site/sources.html#memory),
[`problems.html#p2`](../../../docs/site/problems.html#p2).
