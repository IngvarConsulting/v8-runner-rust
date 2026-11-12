---
id: INV.USE-CASES.A-MISSING-VERSION-FILE-IS-KNOWN-BEFORE-THE-PLATFORM-STARTS
check:
  - src/use_cases/dump_config.rs::an_incremental_dump_without_a_version_file_runs_full_before_the_platform_starts
  - src/use_cases/dump_config.rs::ibcmd_dump_incremental_goes_by_the_runner_copy_through_a_stage
  - src/use_cases/dump_config.rs::dump_incremental_edt_without_a_version_file_in_the_snapshot_is_full
  - src/use_cases/dump_config.rs::an_unrecognized_format_turns_the_dump_full_and_a_version_is_foreign_only_by_measurement
  - src/platform/dump_format.rs::the_version_is_read_from_the_root_attribute_only
  - tests/cli_dump_agent.rs::a_directory_without_a_version_file_is_dumped_full_without_the_generation_skip
  - tests/cli_pull_memory.rs::a_full_dump_over_the_directory_names_the_memory_it_does_not_write
---

# Отсутствие файла версий известно до первой рабочей команды платформе

Выгрузка по изменившемуся без годной копии описи раннера
(`INV.USE-CASES.THE-VERSION-FILE-LIVES-ONLY-IN-THE-RUNNER-MEMORY`) или с копией, в корне
которой раннер не нашёл версии формата (атрибута `version`), переводится в полную до первой
рабочей команды платформе: живой агент к этому времени может уже подниматься
(`INV.PLATFORM.A-LIVE-AGENT-STARTS-WHILE-THE-COMMAND-PREPARES`), но `-update`, `--update` и
`--base` в аргументах не появляются, ответ называет режим `FULL` и причину. Тот же режим
называет превью, которое ничего не пишет (`INV.CLI.PREVIEW-DISPATCHES-NOTHING`).

Полная выгрузка вместо выгрузки по изменившемуся идёт в стадию и ложится поверх каталога
набора, лишнего в нём не удаляет (`INV.CLI.PULL-LAYS-THE-DUMP-OVER-THE-DIRECTORY`), хеш-память
не пишет, и ответ это называет вместе с советом `pull <SET> --force`; снимок формата EDT
заменяется целиком. Полную выгрузку `ibcmd` поверх каталога держит
`INV.USE-CASES.AN-IBCMD-FULL-DUMP-LANDS-OVER-THE-DIRECTORY-THROUGH-A-STAGE`.
Чужую версию формата держит `INV.USE-CASES.A-FOREIGN-FORMAT-VERSION-TURNS-THE-DUMP-FULL`.
