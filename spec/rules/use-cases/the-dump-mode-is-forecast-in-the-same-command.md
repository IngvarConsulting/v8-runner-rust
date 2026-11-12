---
id: INV.USE-CASES.THE-DUMP-MODE-IS-FORECAST-IN-THE-SAME-COMMAND
check:
  - src/use_cases/dump_config.rs::the_designer_asks_no_forecast
  - tests/cli_dump_agent.rs::the_agent_asks_no_forecast
  - src/use_cases/dump_config.rs::a_dump_that_rewrote_every_file_is_full_by_fact
  - src/use_cases/dump_config.rs::a_partly_rewritten_dump_is_incremental
  - src/use_cases/dump_config.rs::a_stray_file_keeps_the_dump_incremental
  - src/use_cases/dump_config.rs::an_empty_scope_is_not_full
  - src/use_cases/dump_config.rs::a_failed_listing_reports_the_mode_as_unknown
  - src/use_cases/dump_config.rs::an_edt_dump_decides_its_mode_on_the_designer_mirror
  - src/use_cases/dump_config.rs::a_set_without_memory_still_decides_by_fact
  - src/change_detection/partial_load.rs::a_dump_file_belongs_to_the_deepest_element_its_path_names
  - src/change_detection/analyzer.rs::only_the_files_of_dropped_elements_are_named
  - src/change_detection/analyzer.rs::a_dropped_part_of_a_staying_element_takes_its_files
  - src/platform/dump_format.rs::the_dump_elements_are_read_at_every_depth
  - tests/cli_agent_standalone.rs::an_sftp_dump_removes_the_files_of_removed_elements
  - tests/cli_agent_standalone.rs::an_sftp_dump_keeps_stray_files
  - tests/cli_agent_standalone.rs::a_dump_removes_the_file_of_a_dropped_part_and_keeps_the_object
  - tests/cli_agent_standalone.rs::the_memory_forgets_the_files_an_sftp_dump_removed
  - tests/cli_agent_standalone.rs::an_sftp_dump_without_a_readable_prior_description_deletes_nothing
  - tests/cli_agent_standalone.rs::an_sftp_dump_keeps_an_element_renamed_by_letter_case
  - tests/cli_agent_standalone.rs::an_sftp_dump_with_an_empty_new_description_deletes_nothing
  - tests/cli_agent_standalone.rs::an_sftp_dump_with_an_unreadable_new_description_deletes_nothing
  - src/use_cases/dump_config/landing.rs::a_failed_removal_names_what_is_left
  - tests/cli_pull_memory.rs::a_file_removed_by_a_pull_by_changes_is_forgotten
  - src/platform/dump_forecast.rs::the_export_status_is_read_by_its_measured_keys
  - src/use_cases/dump_config.rs::an_ibcmd_full_forecast_dumps_through_the_stage_without_sync
  - src/use_cases/dump_config.rs::an_ibcmd_partial_dump_follows_a_full_forecast_too
  - src/use_cases/dump_config.rs::a_dump_without_a_version_file_asks_for_no_forecast
  - tests/cli_push_generation.rs::a_designer_pull_with_an_unchanged_generation_dumps_nothing
  - tests/cli_dump_agent.rs::an_unchanged_generation_skips_an_incremental_dump
  - tests/cli_dump_agent.rs::a_generation_of_another_tool_skips_a_dump_and_a_record_without_a_tool_does_not
---

# Режим выгрузки называет то, что выгрузка сделала

Конфигуратор и агент прогноза перед выгрузкой по изменившемуся не спрашивают
(`-getChanges` / `--get-changes` не передаются): режим, который случился, раннер узнаёт по
факту. Перед выгрузкой по копии описи он отмечает её начало по часам файловой системы
каталога, куда выгрузка ляжет (у EDT — зеркала Конфигуратора), а после успешной выгрузки в стадию и
переноса написанного в каталог перечисляет этот каталог в области обхода набора, не читая файлов.
Если в области есть хоть один файл и каждый переписан (время изменения не раньше начала),
ответ — `FULL` с причиной `platform_rewrote_all`; иначе — `INCREMENTAL`. `ConfigDumpInfo.xml`
в область не входит: в каталог набора опись не попадает. Так платформа поступает и при
`FullDump`, и когда сменились версии всех объектов — полная загрузка, новый объект через
`Configuration.xml` ([замер](../../../references/1c/confirmed-runtime-measurements.md)).
Посторонний файл в каталоге набора, который платформа не трогает, оставляет режим
`INCREMENTAL`. На файловой системе с грубой отметкой времени (FAT, 2 с) правка, сделанная перед
самой выгрузкой, может сойти за переписанную — ошибка в сторону `FULL`. Перечень, который
раннер и так делает, чтобы запомнить записанное
(`INV.USE-CASES.A-PULL-BY-CHANGES-RECORDS-WHAT-IT-WROTE`), отвечает и о режиме; второго
обхода нет. Перечень или отметка начала, которые не удались, дают режим `UNKNOWN` с причиной `unknown`; отмена во
время перечня, как и неудачная выгрузка, — режим плана. Выборка объектов и полная выгрузка
поверх каталога (`OverDirectory::Whole`) перечня не делают.

Выгрузка идёт в стадию, а не в каталог набора (`INV.USE-CASES.THE-VERSION-FILE-LIVES-ONLY-IN-THE-RUNNER-MEMORY`):
платформа ничего в каталоге не удаляет, поэтому удаления раннер переносит сам по двум описям —
копии раннера до выгрузки и новой описи из стадии — у всех исполнителей. Владелец файла — самый
глубокий элемент, который раннер узнаёт по пути: объект верхнего уровня (`<Папка типа>/<Имя>.xml`
и каталог `<Имя>/`), его форма, макет, команда, перерасчёт регистра расчёта (`Forms/`,
`Templates/`, `Commands/`, `Recalculations/`) и вложенная подсистема; файл под каталогом `Ext`
владельца принадлежит ещё и его части, которую опись называет `<владелец>.<имя под Ext>`
(`ObjectModule`, `Module`, `CommandModule`, `Help`, `Predefined`, `Rights`, `Form`). Файл
удаляется, если его владелец или его часть были в описи до выгрузки и ни одно имя описи после неё
не совпадает с ними без учёта регистра букв: так `-update` поверх дерева убирал модуль удалённой
команды, а элемент оставлял ([замер](../../../references/1c/confirmed-runtime-measurements.md),
#646). Элемент, переименованный только регистром, не пропал. Удаления ищутся до переноса
написанного из стадии, поэтому файлы, которые выгрузка пишет, не задеваются. Посторонние файлы,
файлы, чей владелец не распознан, и части, которых в прежней описи не было, остаются; без
прежней или новой описи, если какая-то из них не разобрана или новая пуста при непустой прежней,
не удаляется ничего, и новая опись копией не становится. Пропавших элементов нет — каталог для
удалений не перечисляется. Удаление не выходит за область обхода набора; файл, который удалить
не удалось, выгрузка называет отказом.

`ibcmd` прогноз по-прежнему спрашивает (`config export status`) в той же команде и под тем
же замком: по нему он выбирает путь. Прогноз «полная» (`modified: all`) отвечает `FULL` с
причиной `platform_forecast` и выгружает полностью
(`INV.USE-CASES.AN-IBCMD-FULL-DUMP-LANDS-OVER-THE-DIRECTORY-THROUGH-A-STAGE`) — так же и
выборка `ibcmd`, которая идёт по копии описи; без копии выборка `ibcmd` полная с причиной
`version_file`. Прогноз, которого нет в замеренном словаре, или
отказ прогноза дают режим `UNKNOWN` с причиной `unknown`, а не «инкрементальный»; выгрузка
при этом идёт как просили. Ответ обещает режим по прогнозу и не больше: базу между прогнозом и
выгрузкой может изменить тот, кого замок раннера не держит.

Выгрузка, которую раннер пропускает по неизменному поколению, ни прогноза, ни перечня не
делает. Без годной копии описи или с чужой выгрузка полная по
`INV.USE-CASES.A-MISSING-VERSION-FILE-IS-KNOWN-BEFORE-THE-PLATFORM-STARTS` и
`INV.USE-CASES.A-FOREIGN-FORMAT-VERSION-TURNS-THE-DUMP-FULL`. Превью платформу не запускает и
называет режим плана. Пару «запрошенный и случившийся режим» несёт форма `CTR.WIRE.PULL-DATA`.
