---
id: INV.USE-CASES.A-BASE-NAMED-BY-A-CONNECTION-STRING-IS-REMEMBERED-BY-THE-STRING
check:
  - src/change_detection/source_sets.rs::an_ad_hoc_base_is_remembered_by_its_address_and_empty_sources_skip
  - tests/cli_pull_memory.rs::an_ad_hoc_base_is_remembered_by_its_connection_string
---

# База, названная строкой соединения, помнится по строке

После команды с `--infobase <строка соединения>` раннер ведёт память о базе под
`workPath/infobases/`, как у именованной базы. Каталог памяти называется безопасным ключом,
выведенным из нормализованной строки соединения без учётных данных, и с именами объявленных
баз не пересекается. Привязку памяти к базе держит
`INV.USE-CASES.HASH-MEMORY-IS-SCOPED-TO-ITS-BASE-AND-SOURCE`. Следующая команда с той же
строкой продолжает с этой памятью, а не начинает как первая.

Строку, адрес которой раннер не распознаёт, сравнить не с чем: памяти у такой базы нет.
