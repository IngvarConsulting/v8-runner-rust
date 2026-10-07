---
id: INV.CLI.A-STANDALONE-SOURCE-IS-REFUSED-WITH-A-RECIPE
check: [tests/cli_infobase_copy.rs::a_standalone_source_is_refused_before_the_snapshot]
---

# Источник на автономном сервере — отказ с рецептом

Если источник — база автономного сервера, `infobase create --from` отказывает родом подбора
(`capability`, код `target`) до снимка, и под превью тоже, и называет, как снять образ на
машине сервера и поднять из него базу этой копии: `infobase restore --create`, затем
`push --force`.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).
