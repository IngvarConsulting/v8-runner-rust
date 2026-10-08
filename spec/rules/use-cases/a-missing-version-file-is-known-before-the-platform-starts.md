---
id: INV.USE-CASES.A-MISSING-VERSION-FILE-IS-KNOWN-BEFORE-THE-PLATFORM-STARTS
check:
  - src/use_cases/dump_config.rs::an_incremental_dump_without_a_version_file_runs_full_before_the_platform_starts
  - src/use_cases/dump_config.rs::ibcmd_dump_incremental_uses_sync_against_resolved_target
  - src/use_cases/dump_config.rs::dump_incremental_edt_without_a_version_file_in_the_snapshot_is_full
  - src/use_cases/dump_config.rs::an_unrecognized_format_turns_the_dump_full_and_a_version_is_foreign_only_by_measurement
  - src/platform/dump_format.rs::the_version_is_read_from_the_root_attribute_only
  - tests/cli_dump_agent.rs::a_directory_without_a_version_file_is_dumped_full_without_the_generation_skip
  - tests/cli_pull_memory.rs::a_full_dump_over_the_directory_names_the_memory_it_does_not_write
---

# Отсутствие файла версий известно до запуска платформы

Выгрузка по изменившемуся при отсутствующем у раннера файле версий или при файле, в корне
которого раннер не нашёл версии формата (атрибута `version`), переводится в полную до
запуска платформы: `-update`, `--update` и `--sync` в аргументах не появляются, ответ
называет режим `FULL` и причину. Файл в каталоге при этом берётся таким, каким его оставит
сверка с копией раннера; тот же режим называет превью
(`INV.CLI.PREVIEW-DISPATCHES-NOTHING`). Исключение — восстановление файла версий по
`INV.USE-CASES.A-VERSION-FILE-ALONE-IS-DUMPED-ONLY-WHEN-THE-DIRECTORY-MATCHES-THE-BASE`.

Полная выгрузка вместо выгрузки по изменившемуся ложится поверх каталога набора и лишнего
в нём не удаляет (`INV.CLI.PULL-LAYS-THE-DUMP-OVER-THE-DIRECTORY`), хеш-память не пишет, и
ответ это называет вместе с советом `pull <SET> --force`; снимок формата EDT заменяется
целиком. `ibcmd config export` без `--sync` в непустой каталог отказывает, и полная выгрузка `ibcmd`
поверх каталога не выполняется — [#423](https://github.com/IngvarConsulting/v8-runner-rust/issues/423); выборка `ibcmd` (`--object`, идёт как
`--sync`) без файла версий отказывает так же
([замер](../../../references/1c/confirmed-runtime-measurements.md)).
Чужую версию формата держит `INV.USE-CASES.A-FOREIGN-FORMAT-VERSION-TURNS-THE-DUMP-FULL`.
