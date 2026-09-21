---
id: DEC.2026-09-21.A-PACKAGE-IS-UPLOADED-AND-DOWNLOADED
status: planned
governs: product
realized: null
supersedes: []
superseded-by: null
establishes: [INV.CLI.A-WRONG-PACKAGE-SUFFIX-NAMES-THE-NEIGHBOUR-COMMAND]
changes: [CTR.WIRE.LOAD-DATA, CTR.WIRE.INFOBASE-CONFIGURATION-EXPORT-DATA, INV.CLI.EXPORT-GRAMMAR-IS-FIXED, INV.CLI.EXPORT-SUFFIX-MATCHES-THE-SUBJECT]
---

# Пакет отдают базе `upload` и забирают `download`

**Решение.** Пакет `.cf`/`.cfe` отдают базе командой `upload <файл>` и забирают командой
`download`; пара названа по направлению, как `tools download` забирает дистрибутив.
`upload` заменяет основную конфигурацию целиком; `upload --mode combine` объединяет с
пакетом по файлу настроек, `--mode update` обновляет конфигурацию поставщика. Режим
назван `combine`, а не `merge`: слияния версий здесь нет, `merge` обещало бы
трёхстороннее слияние. `download` забирает основную конфигурацию, `download --state db`
— конфигурацию базы данных. Команда отказывает по расширению файла и называет соседку:
`infobase dump --output main.cf` — это `download`, `upload ib.dt` — это `infobase
restore`.

**Почему.** Слово платформы `load` пары не имеет; `save` читалось бы как запись внутрь
базы; `export` у платформы занят выгрузкой в XML; `checkout` в гите — местное действие,
а загрузка пакета меняет базу.

**Не затрагивает.** `DEC.2026-09-02.EXPORT-INTENTS-ARE-TYPED-SEPARATELY`: намерений
по-прежнему два — пакет конфигурации и образ базы остаются разными командами с разными
расширениями.

Источник: [`cli.html#map`](../../../docs/site/cli.html#map),
[`cli.html#notgit`](../../../docs/site/cli.html#notgit).
