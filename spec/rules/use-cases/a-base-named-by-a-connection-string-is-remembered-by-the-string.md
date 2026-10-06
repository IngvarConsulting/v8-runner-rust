---
id: INV.USE-CASES.A-BASE-NAMED-BY-A-CONNECTION-STRING-IS-REMEMBERED-BY-THE-STRING
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/214
---

# База, названная строкой соединения, помнится по строке

После команды с `--infobase <строка соединения>` раннер ведёт память о базе под
`workPath/infobases/`, как у именованной базы. Каталог памяти называется безопасным ключом,
выведенным из нормализованной строки соединения без учётных данных, и с именами объявленных
баз не пересекается. Привязку памяти к базе держит
`INV.USE-CASES.HASH-MEMORY-IS-SCOPED-TO-ITS-BASE-AND-SOURCE`. Следующая команда с той же
строкой продолжает с этой памятью, а не начинает как первая.

Сегодня такая команда памяти не читает и не пишет
(`src/change_detection/source_sets.rs::ad_hoc_analysis_never_reads_or_writes_memory_and_empty_sources_skip`,
`tests/cli_pull_memory.rs::an_ad_hoc_base_never_uses_the_named_hash_baseline`); эти проверки
меняются вместе с #214.
