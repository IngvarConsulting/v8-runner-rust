---
id: INV.USE-CASES.FULL-PULL-RECORDS-THE-PUBLISHED-TREE
check:
  - tests/cli_pull_memory.rs::first_full_pull_establishes_the_baseline_for_all_exporters
  - tests/cli_pull_memory.rs::full_pull_replacing_a_source_symlink_records_the_published_directory_identity
  - tests/cli_pull_memory.rs::full_pull_replaces_a_previous_push_baseline
  - tests/cli_pull_memory.rs::full_pull_repairs_corrupt_hash_memory
  - tests/cli_pull_memory.rs::git_refusal_preserves_memory_and_a_retry_can_publish
  - src/use_cases/dump_config.rs::cancelled_full_publication_leaves_source_and_memory_unchanged_and_can_retry
  - src/use_cases/dump_config.rs::full_publication_reports_memory_write_failure_after_publishing
  - src/use_cases/dump_config.rs::a_staging_scan_failure_does_not_discard_the_successful_full_dump
  - src/use_cases/dump_config.rs::full_publication_rechecks_that_the_target_does_not_contain_work_path
  - src/change_detection/analyzer.rs::publishing_a_prepared_snapshot_does_not_absorb_a_later_user_edit
  - tests/architecture_guardrails.rs::a_full_pull_records_the_staged_tree_it_publishes
---

# Полная выгрузка записывает опубликованное дерево

Успешный полный `pull` в формате `DESIGNER` через Конфигуратор, `ibcmd` или агент
записывает хеши выгруженного дерева для выбранной именованной базы и набора. Следующий
`push` без правок пропускает загрузку. Полная выгрузка агента не пропускается по
журналу поколений: она восстанавливает и дерево, и хеш-память.

Хеши готовятся по staging до публикации, сохраняются после её успеха. Привязка
соответствует получившемуся каталогу, в том числе когда публикация заменяет символическую
ссылку каталогом; содержимое опубликованного дерева повторно не сканируется. Отказ публикации
не меняет память. Ошибка обхода staging или записи памяти не отбрасывает успешную
выгрузку: ответ называет опубликованные исходники, несохранённую память и повторный
полный `pull` как следующий шаг. Загрузка старых исходников не является восстановлением
неудавшейся выгрузки. Правка дерева после публикации остаётся изменением.

Защита от возврата: причина #53 — выгрузка меняла дерево, сохраняя прежний снимок.
Владелец подготовки и записи снимка — `change_detection::analyzer`, область памяти —
`SourceSetContext`, общая публикация полной выгрузки — `dump_config::publish_full_dump`.
Сквозная проверка держит всех исполнителей. Страж кода держит, что выгрузку исходников
публикует только `publish_full_dump` (и EDT-выгрузка со своим путём), что он хеширует
staging до публикации и пишет память после неё и что опубликованное дерево он заново не
обходит.
