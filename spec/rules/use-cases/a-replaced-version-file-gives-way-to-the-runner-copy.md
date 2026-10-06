---
id: INV.USE-CASES.A-REPLACED-VERSION-FILE-GIVES-WAY-TO-THE-RUNNER-COPY
check:
  - tests/cli_pull_memory.rs::a_foreign_version_file_between_commands_does_not_reach_the_dump
  - tests/cli_pull_memory.rs::an_ibcmd_partial_pull_dumps_from_the_runner_copy
  - tests/cli_pull_memory.rs::a_push_refreshes_the_runner_copy
  - tests/cli_pull_memory.rs::an_agent_push_loads_over_the_runner_copy_and_records_the_new_one
  - src/use_cases/version_file.rs::a_replaced_file_gives_way_to_the_runner_copy
  - src/use_cases/version_file.rs::a_missing_file_is_not_restored
  - src/use_cases/version_file.rs::a_copy_of_another_pair_is_not_restored
  - src/use_cases/version_file.rs::an_interrupted_identity_change_leaves_no_pair_owning_the_copy
  - src/use_cases/version_file.rs::a_restore_removes_temporary_files_left_by_a_killed_write
  - src/change_detection/scanner.rs::service_and_generated_paths_are_never_scanned
  - src/support/fs.rs::an_atomic_write_keeps_the_mode_and_leaves_no_candidate
---

# Подменённый файл версий уступает копии раннера

У набора формата Конфигуратора с памятью именованной базы раннер держит копию
`ConfigDumpInfo.xml` в `workPath/infobases/<имя базы>/dump-info/<набор>/` вместе с
тождеством памяти набора. Перед работой от файла версий — выгрузкой по изменившемуся,
выборкой `ibcmd`, которую тот выгружает как `--sync`, и загрузкой `push` Конфигуратором или
агентом — раннер сверяет отпечаток файла в каталоге набора с отпечатком копии и
подменённый файл заменяет копией до запуска платформы. Замена атомарна и сохраняет права
прежнего файла; временный файл замены назван по цели, оставленный снятым процессом
убирается перед следующей сверкой и в обход изменений не входит.

Копия, чьё тождество не совпадает с тождеством набора или стёрто оборванной сменой, своей
не считается и не подкладывается. Если файла версий в каталоге нет, копия тоже не
подкладывается: каталог без описи она не подтверждает. Изменения при этом не теряются —
выгрузка с `-update` без файла версий отказывает
([замер](../../../references/1c/confirmed-runtime-measurements.md)).
