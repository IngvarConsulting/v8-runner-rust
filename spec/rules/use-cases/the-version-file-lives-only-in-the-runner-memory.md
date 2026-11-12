---
id: INV.USE-CASES.THE-VERSION-FILE-LIVES-ONLY-IN-THE-RUNNER-MEMORY
check:
  - src/use_cases/version_file.rs::the_staged_description_becomes_the_copy_and_never_reaches_the_sources
  - src/use_cases/version_file.rs::a_recreated_source_directory_does_not_confirm_the_copy
  - src/use_cases/version_file.rs::a_copy_of_another_pair_is_not_used
  - src/use_cases/version_file.rs::an_interrupted_identity_change_leaves_no_pair_owning_the_copy
  - src/use_cases/version_file.rs::a_copy_left_by_an_earlier_runner_is_adopted_only_beside_the_same_file
  - src/use_cases/version_file.rs::a_full_dump_takes_the_description_out_of_the_stage_before_publication
  - src/use_cases/dump_config.rs::a_dump_by_changes_lands_from_the_stage_and_keeps_the_description_in_memory
  - src/use_cases/dump_config.rs::ibcmd_dump_incremental_goes_by_the_runner_copy_through_a_stage
  - tests/cli_pull_memory.rs::a_version_file_in_the_sources_never_reaches_the_dump
  - tests/cli_pull_memory.rs::a_recloned_source_directory_is_pulled_full
  - tests/cli_pull_memory.rs::a_copy_of_an_earlier_runner_is_adopted_beside_the_same_file
  - tests/cli_pull_memory.rs::an_ibcmd_partial_pull_dumps_from_the_runner_copy
  - tests/cli_dump_agent.rs::incremental_mode_dumps_into_a_stage_through_a_link_and_lays_over_the_target
  - tests/cli_agent_standalone.rs::an_incremental_dump_through_sftp_sends_only_the_dump_info
  - tests/cli_external_full_pull.rs::an_edit_after_publication_is_not_absorbed_into_the_prepared_baseline
  - tests/cli_dump.rs::a_full_dump_replaces_an_ignored_version_file_without_asking
  - tests/cli_dump.rs::a_version_file_tracked_in_the_sources_is_left_alone
  - tests/cli_build.rs::a_push_does_not_refuse_a_version_file_tracked_by_git
  - tests/cli_diff.rs::version_file_is_resolved_without_rewriting_the_sources
---

# Файл версий живёт только в памяти раннера

`ConfigDumpInfo.xml` набора с памятью базы лежит только в
`localPath/infobases/<имя базы>/dump-info/<набор>/` — копией раннера рядом с тождеством пары
«база ↔ набор» и свидетельством каталога набора (его идентификатором в файловой системе). В
каталог набора раннер описи не кладёт. Выгрузка по изменившемуся, выборка `ibcmd` и полная
выгрузка поверх каталога пишут в пустой каталог стадии под `workPath`, по копии описи
(`-update -configDumpInfoForChanges` у Конфигуратора, `--config-dump-info-for-changes` у агента
с общим каталогом, `--update` в каталог с описью у агента по SFTP, `config export --base` у
`ibcmd`); в каталог набора переносится всё, кроме описи, а опись становится новой копией. Полная
выгрузка публикует дерево без описи. `diff` и прогноз `ibcmd` берут опись из копии. У формата
EDT каталог набора — снимок Конфигуратора той же базы.

Копия годится, только если тождество её пары наше и свидетельство совпадает с каталогом
набора: каталог, удалённый и созданный заново (новый клон), её не подтверждает, и выгрузка по
изменившемуся становится полной до первой рабочей команды платформе
(`INV.USE-CASES.A-MISSING-VERSION-FILE-IS-KNOWN-BEFORE-THE-PLATFORM-STARTS`). Копия, чьё
тождество чужое или стёрто оборванной записью, тоже не годится. У набора без памяти базы копии
нет, и выгрузка по изменившемуся всегда полная.

`ConfigDumpInfo.xml`, оставшийся в каталоге набора от прежних версий раннера, раннер не читает и
не пишет: ни сверкой, ни отказом «файл в индексе гита» — его нет; выгрузка поверх каталога его
не трогает, а полная выгрузка переносит в новое дерево как был. Исключение одно — переход со
старой памяти: копию прежней версии, у которой нет свидетельства, раннер один раз сравнивает с
этим файлом, и если они равны байт в байт, записывает свидетельство и считает копию годной;
иначе первая выгрузка полная. Превью и `diff` при этом ничего не пишут.
