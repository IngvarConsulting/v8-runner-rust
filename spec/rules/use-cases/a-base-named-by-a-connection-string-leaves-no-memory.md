---
id: INV.USE-CASES.A-BASE-NAMED-BY-A-CONNECTION-STRING-LEAVES-NO-MEMORY
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/214
---

# База, названная строкой соединения, памяти не оставляет

После команды с `--infobase <строка соединения>` под `workPath/infobases/` не появляется
ни каталога, ни записи, и следующая команда с той же строкой идёт как первая: `pull`
полный, `push` в непустую базу, которая не общая и которую не держит другая рабочая копия, —
отказ с выходами `pull` и `push --force`.

Проверенный срез — отсутствие чтения и записи хешов обмена с базой:
`src/change_detection/source_sets.rs::ad_hoc_analysis_never_reads_or_writes_memory_and_empty_sources_skip`
и `tests/cli_pull_memory.rs::an_ad_hoc_base_never_uses_the_named_hash_baseline`.
Журнал поколений агента, файл версий и отказ отправки в непустую базу остаются в #214/#217.
