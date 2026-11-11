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
  - src/use_cases/source_changes.rs::a_full_pull_remembers_the_published_tree_as_git_add_would_store_it
  - src/use_cases/source_changes.rs::an_edit_after_a_full_pull_was_published_stays_a_change
---

# Полная выгрузка записывает опубликованное дерево

Успешный полный `pull` со ступенчатой публикацией (`pull --force`) в формате `DESIGNER`
через Конфигуратор, `ibcmd` или агент записывает хеши выгруженного дерева для выбранной базы
и набора. Полная выгрузка поверх каталога вместо выгрузки по изменившемуся хешей не пишет
(`INV.USE-CASES.A-MISSING-VERSION-FILE-IS-KNOWN-BEFORE-THE-PLATFORM-STARTS`). Следующий
`push` без правок пропускает загрузку. Полная выгрузка агента не пропускается по
журналу поколений: она восстанавливает и дерево, и хеш-память.

Хеши готовятся по staging до публикации, сохраняются после её успеха. У набора в рабочей
копии git после успешной публикации раннер спрашивает git по корню набора (`git status` с
необязательной блокировкой): переписанные файлы git перечитывает и обновляет индекс в этой
команде, а не в следующей, и хеши берутся такими, какими их сохранил бы `git add` (#575).
Запоминается файл, время которого то же, что было в staging: публикация переносит файлы, не
переписывая их, и правка после публикации остаётся изменением. Коммиты, ветки и содержимое
индекса не меняются; отказ git выгрузку не отменяет — тогда пишутся хеши байтов staging.
Превью ничего не публикует и индекса не трогает. Привязка соответствует получившемуся
каталогу, в том числе когда публикация заменяет символическую ссылку каталогом; сам раннер
опубликованное дерево заново не обходит. Отказ публикации
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
