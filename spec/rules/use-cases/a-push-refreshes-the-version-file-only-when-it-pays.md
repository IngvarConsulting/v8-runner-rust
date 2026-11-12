---
id: INV.USE-CASES.A-PUSH-REFRESHES-THE-VERSION-FILE-ONLY-WHEN-IT-PAYS
check:
  - src/use_cases/build_project/coordinator.rs::a_full_or_a_large_matched_load_refreshes_the_version_file
  - src/use_cases/build_project/coordinator.rs::modules_and_descriptions_count_alike
  - src/use_cases/build_project/coordinator.rs::a_change_the_platform_dumps_in_full_refreshes_the_version_file
  - src/change_detection/partial_load.rs::a_load_list_names_what_the_next_dump_takes_back
  - src/change_detection/partial_load.rs::a_deleted_object
  - src/change_detection/partial_load.rs::a_renamed_object
  - src/change_detection/partial_load.rs::a_renamed_configuration
  - src/change_detection/partial_load.rs::changed_predefined_items
  - src/change_detection/partial_load.rs::a_changed_flowchart
  - src/change_detection/partial_load.rs::and_an_added_object_counts_too
  - src/change_detection/partial_load.rs::but_not_a_changed_object_description
  - src/change_detection/partial_load.rs::but_not_a_changed_module_or_form
  - src/use_cases/version_file.rs::unknown_removals_are_remembered_until_the_next_copy_is_recorded
  - tests/cli_push_generation.rs::a_partial_push_after_a_pull_with_unknown_removals_leaves_the_copy_and_says_so
  - tests/cli_push_generation.rs::a_full_push_after_a_pull_with_unknown_removals_refreshes_the_copy_and_clears_the_mark
  - src/use_cases/dump_config.rs::a_left_version_file_dir_is_removed_only_from_the_stage_root
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
загрузки основной конфигурации в любом из случаев:

1. загрузка была полной;
2. загрузка по изменившемуся, после которой выгрузка по прежней описи заведомо пошла бы полной.
   По документации платформы (8.5.1, 2.17.4) это удаление и переименование объекта,
   переименование конфигурации, изменение `Ext/Predefined.xml` или `Ext/Flowchart.xml`. Признак
   берётся из того же списка частичной загрузки, что грузится: удаление и переименование объекта
   и переименование конфигурации частичная загрузка принимает только с корневым
   `Configuration.xml` в списке, предопределённые и схема — файлы `Ext/Predefined.xml` и
   `Ext/Flowchart.xml`. Корневой `Configuration.xml` попадает в список и при добавлении объекта,
   изменении каталога объекта и любой правке свойств конфигурации (например, `Version`); опись
   тогда освежается сверх документированного, ценой одного её запроса;
3. загрузка по изменившемуся не меньше 1000 файлов: модули, описания объектов и форм и прочие
   файлы списка считаются одной мерой.

Во втором и третьем случае поколение набора перед загрузкой должно совпасть с записанным: иначе
база могла нести правки, которых нет в каталоге, и свежая опись заявила бы их выгруженными.
Поколение читается до выгрузки описи и после, тем же инструментом, и должно совпасть. Иначе —
сдвиг поколения, нет ответа, сбой, отмена, или перед загрузкой по изменившемуся последняя
выгрузка не узнала удалений (отметка в памяти набора, её снимает следующая записанная копия;
после полной загрузки база равна каталогу, свежая опись верна, и отметку снимает её запись) —
копия остаётся прежней, ответ это называет, а загрузка остаётся удачной; молчит только набор без журнала поколений. У расширения
отдельная выгрузка описи не замерена ни у одного исполнителя, и его копия загрузкой не
трогается. Прочая загрузка копию не трогает: копия старше базы ровно на загруженное, и следующая
выгрузка по изменившемуся выгрузит эти объекты заново, ничего не пропустив.

Свежая опись — опись платформы. Дерево, выгруженное в прежнем формате (например, 2.17 при
платформе с форматом 2.20), после неё остаётся смешанным, пока полный `pull` его не выровняет;
это допустимо: выгрузка по изменившемуся пишет загруженное в формате платформы.

## Замер

На КА 8.3.27 с живым агентом запрос одной описи стоил отправке 1,6–1,7 с после модулей и 6–7 с
после описаний объектов, а `pull` после отправки до 300 модулей или описаний без запроса дороже
лишь на 0,8–1,7 с. Обратная выгрузка у дерева КА идёт около 5 мс на файл: с тысячи модулей она
дороже запроса, у тысячи описаний (около 5 с) — сравнима с ним; порог один для всех файлов.
Удаление объекта выгрузка по прежней описи делает полной (малая база, четыре
версии платформы); на КА после удаления общего модуля и свежей описи `pull` пропустил выгрузку
(1,6 с). После полной загрузки версии всех объектов меняются, и следующая выгрузка по старой
описи была бы полной ([замер](../../../references/1c/confirmed-runtime-measurements.md), раздел
«Свежий файл версий после крупной загрузки (#646)»).
