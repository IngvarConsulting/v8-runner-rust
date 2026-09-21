---
id: DEC.2026-09-21.COMMANDS-FOLLOW-THE-GIT-VOCABULARY
status: planned
governs: product
realized: null
supersedes: []
superseded-by: null
establishes: [INV.CLI.A-POSITIONAL-ARGUMENT-NEVER-NAMES-A-BASE, INV.CONFIG.A-PROVIDERS-KEY-IS-NAMED-AFTER-ITS-COMMAND]
changes: [CTR.WIRE.BUILD-DATA, CTR.WIRE.DUMP-DATA, CTR.WIRE.LOAD-DATA, CTR.WIRE.INFOBASE-CONFIGURATION-EXPORT-DATA, CTR.WIRE.CONFIG-INIT-DATA, CTR.WIRE.BOOTSTRAP-DATA, CTR.WIRE.SYNTAX-DATA, CTR.WIRE.INIT-DATA, CTR.WIRE.TEST-DATA]
---

# Команды называются словарём гита

**Решение.** Имя команды берётся из словаря гита только там, где знающий гит человек
получит ожидаемое поведение: незнакомое поведение под знакомым именем опаснее
незнакомого имени. Поверхность командной строки: `status`, `init`, `clone`,
`infobase create`, `push`, `apply`, `reset`, `pull`, `upload`, `download`, `make`,
`check`, `test`, `infobase dump`, `infobase restore`, `sessions`, `extensions`,
`publish`, `launch`, `convert`, `tools download`, `mcp serve`, `version`. Ключ выбора
исполнителя называется именем команды (`providers.push`, `providers.infobase.create`); у
`publish`, `check` и `sessions` ключа нет. Позиционный аргумент никогда не называет базу
и не разбирается как строка соединения; у команд, принимающих набор исходников,
позиционный — набор (`push my-ext`, `make my-ext --output …`), базу называет только
`--infobase`. Имена инструментов MCP — отдельный контракт и за командной строкой не
следуют.

**Почему.** У командной строки три потребителя — сценарии сборки, человек и Unica, — и
всем троим нужно понимать, что делает команда, до чтения справки; словарь гита они уже
знают. Позиционный аргумент, который может оказаться и ссылкой, и базой, заставил бы
команду гадать.

**Цена.** Переименование меняет значение `command` в конверте и символ контракта
`CTR.WIRE.*-DATA`: при реализации контракты заводятся под новыми символами с
перепорождёнными артефактами, а решения-владельцы прежних получают преемников. Таблица
переименований и судьба прежних имён — в
`DEC.2026-09-21.OLD-NAMES-ARE-HIDDEN-SYNONYMS-FOR-ONE-CYCLE`.

**Не затрагивает.** `CTR.MCP.PUBLISHED-TOOL-SURFACE`: имена и состав инструментов MCP не
меняются.

Источник: [`cli.html#why`](../../../docs/site/cli.html#why),
[`cli.html#map`](../../../docs/site/cli.html#map),
[`cli.html#keys`](../../../docs/site/cli.html#keys).
