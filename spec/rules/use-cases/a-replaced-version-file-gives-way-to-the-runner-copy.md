---
id: INV.USE-CASES.A-REPLACED-VERSION-FILE-GIVES-WAY-TO-THE-RUNNER-COPY
check:
  - tests/cli_pull_memory.rs::a_foreign_version_file_between_commands_does_not_reach_the_dump
  - tests/cli_pull_memory.rs::an_ibcmd_partial_pull_dumps_from_the_runner_copy
  - tests/cli_pull_memory.rs::a_push_refreshes_the_runner_copy
  - tests/cli_pull_memory.rs::an_agent_push_loads_over_the_runner_copy_and_records_the_new_one
  - tests/cli_pull_memory.rs::a_source_edit_keeps_the_pull_incremental
  - src/use_cases/version_file.rs::a_replaced_file_gives_way_to_the_runner_copy
  - src/use_cases/version_file.rs::a_missing_file_is_not_restored
  - src/use_cases/version_file.rs::a_copy_of_another_pair_is_not_restored
  - src/use_cases/version_file.rs::an_interrupted_identity_change_leaves_no_pair_owning_the_copy
---

# Подменённый файл версий уступает копии раннера

У набора с памятью базы раннер держит копию `ConfigDumpInfo.xml` в
`workPath/infobases/<имя базы>/dump-info/<набор>/` вместе с тождеством памяти набора; у
формата EDT файл версий лежит в снимке Конфигуратора под памятью той же базы. Перед работой от файла версий — выгрузкой по изменившемуся,
выборкой `ibcmd` (`--object`), которую тот выгружает как `--sync`, и загрузкой `push`
Конфигуратором или агентом — раннер сверяет отпечаток файла в каталоге набора с
отпечатком копии и подменённый файл заменяет копией до запуска платформы. Сверяется только
сам файл версий: правка исходников в каталоге выгрузку по изменившемуся полной не делает.

Копия, чьё тождество не совпадает с тождеством набора или стёрто оборванной сменой, своей
не считается и не подкладывается. Если файла версий в каталоге нет, копия тоже не
подкладывается: каталог без описи она не подтверждает. Выгрузка по изменившемуся без файла
версий становится полной поверх каталога до запуска платформы
(`INV.USE-CASES.A-MISSING-VERSION-FILE-IS-KNOWN-BEFORE-THE-PLATFORM-STARTS`), и
незакоммиченное в каталоге её останавливает
(`INV.USE-CASES.REPLACING-A-USER-DIRECTORY-ASKS-FIRST`).
