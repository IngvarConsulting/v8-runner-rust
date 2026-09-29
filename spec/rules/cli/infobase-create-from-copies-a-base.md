---
id: INV.CLI.INFOBASE-CREATE-FROM-COPIES-A-BASE
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/330
---

# `infobase create --from` копирует базу

`infobase create --from <база>` создаёт базу этой рабочей копии как копию другой базы, с её
данными и конфигурацией: снимает образ с источника и создаёт из него файловую базу через
`ibcmd`, базу в кластере — Конфигуратором по шаблону .dt. Новую базу на автономном сервере
команда не создаёт: отказ называет рецепт.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies),
[`cli.html#map`](../../../docs/site/cli.html#map).
