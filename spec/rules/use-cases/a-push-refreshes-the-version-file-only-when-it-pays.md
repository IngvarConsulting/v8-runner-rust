---
id: INV.USE-CASES.A-PUSH-REFRESHES-THE-VERSION-FILE-ONLY-WHEN-IT-PAYS
check:
  - src/use_cases/build_project/coordinator.rs::a_full_or_a_costly_matched_load_refreshes_the_version_file
  - src/use_cases/build_project/coordinator.rs::a_description_costs_more_than_a_module
  - src/use_cases/build_project/coordinator.rs::a_change_the_platform_dumps_in_full_refreshes_the_version_file
  - src/change_detection/partial_load.rs::a_load_list_names_what_the_next_dump_takes_back
  - src/use_cases/version_file.rs::unknown_removals_are_remembered_until_the_next_copy_is_recorded
  - tests/cli_push_generation.rs::a_push_after_a_pull_with_unknown_removals_leaves_the_copy_and_says_so
  - tests/cli_push_generation.rs::a_push_without_the_generation_after_the_load_says_the_copy_was_not_refreshed
  - tests/cli_push_generation.rs::a_full_push_refreshes_the_runner_copy
  - tests/cli_push_generation.rs::a_version_file_dumped_while_the_base_changed_is_not_recorded
  - tests/cli_push_generation.rs::a_full_ibcmd_push_refreshes_the_runner_copy_and_the_next_pull_goes_by_it
  - tests/cli_pull_memory.rs::a_push_leaves_the_runner_copy_and_passes_no_update_key
  - tests/cli_pull_memory.rs::an_agent_full_push_passes_no_update_key_and_refreshes_the_copy_separately
  - tests/cli_pull_memory.rs::an_agent_partial_push_passes_no_update_key_and_leaves_the_copy
  - tests/cli_pull_memory.rs::a_failed_push_does_not_change_the_runner_copy
  - tests/cli_build_agent.rs::a_managed_build_loads_and_updates_in_one_session_and_records_the_generation
---

# Загрузка получает свежий файл версий, только когда это окупается

`push` не передаёт загрузке ключ обновления описи (`-updateConfigDumpInfo` у Конфигуратора,
`--update-config-dump-info` у агента): платформа писала бы опись в каталог исходников. Свежую
опись загрузка получает отдельной выгрузкой одной описи во временный каталог под `workPath`
(`-configDumpInfoOnly`, `--config-dump-info-only`, `ibcmd config export info`) и делает её копией
раннера (`INV.USE-CASES.THE-VERSION-FILE-LIVES-ONLY-IN-THE-RUNNER-MEMORY`) только после удачной
загрузки основной конфигурации — полной или по изменившемуся, перед которой поколение набора
совпало с записанным и после которой выгрузка по прежней описи была бы полной или дорогой.
Полной она была бы, если в списке частичной загрузки (тот же список, что грузится) есть
корневой `Configuration.xml` — у удалённого и переименованного объекта он там всегда, — или
`Ext/Predefined.xml`, `Ext/Flowchart.xml`. Дорогой — если цена загруженного (модуль `.bsl` — 1,
другой файл — цена описания, константы в коде с замером) достигла порога. Поколение читается до
выгрузки описи и после, тем же инструментом, и должно совпасть. Иначе — сдвиг поколения, нет
ответа, сбой, отмена, или последняя выгрузка не узнала удалений (отметка в памяти набора, её
снимает следующая записанная копия) — копия остаётся прежней, ответ это называет, а загрузка
остаётся удачной; молчит только набор без журнала поколений. У расширения отдельная выгрузка
описи не замерена ни у одного исполнителя, и его копия загрузкой не трогается. Небольшая загрузка
копию не трогает: копия старше базы ровно на загруженное, и следующая выгрузка по изменившемуся
выгрузит эти объекты заново, ничего не пропустив.

Свежая опись — опись платформы. Дерево, выгруженное в прежнем формате (например, 2.17 при
платформе с форматом 2.20), после неё остаётся смешанным, пока полный `pull` его не выровняет;
это допустимо: выгрузка по изменившемуся пишет загруженное в формате платформы.

## Замер

Запрос одной описи стоил отправке на КА 1,6–1,7 с, а выгрузка загруженного следующим `pull`
после 2, 30 и 300 общих модулей — 1,1–1,7 с; выгрузка дерева КА идёт около 5 мс на файл, и с
тысячи файлов обратная выгрузка однозначно дороже. После полной загрузки версии всех объектов
меняются, и следующая выгрузка по старой описи была бы полной
([замер](../../../references/1c/confirmed-runtime-measurements.md), раздел «Свежий файл версий
после крупной загрузки (#646)»).
