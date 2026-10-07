---
id: INV.CLI.A-FILE-BASE-OF-AN-EDT-PROJECT-IS-ASSEMBLED-FROM-ITS-SOURCES
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/204
---

# Файловая база проекта EDT собирается из его исходников

`infobase create` в проекте формата EDT создаёт файловую базу сразу с основной
конфигурацией: исходники EDT переводятся в XML и загружаются при создании, а память
записывает собранный набор, как у проекта формата Конфигуратора
(`INV.CLI.INFOBASE-CREATE-FOLLOWS-THE-TARGET-KIND`).

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).
