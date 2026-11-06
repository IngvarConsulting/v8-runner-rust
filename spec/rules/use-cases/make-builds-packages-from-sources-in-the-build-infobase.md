---
id: INV.USE-CASES.MAKE-BUILDS-PACKAGES-FROM-SOURCES-IN-THE-BUILD-INFOBASE
check:
  - src/use_cases/artifacts.rs::designer_builds_a_cf_from_the_sources_in_the_build_infobase
  - src/use_cases/artifacts.rs::a_dump_that_resized_the_database_publishes_the_package_and_drops_the_memory
  - src/use_cases/artifacts.rs::another_platform_rebuilds_the_build_infobase
  - src/use_cases/artifacts.rs::another_edt_installation_changes_the_identity_of_the_translation
  - src/use_cases/build_infobase.rs::a_full_build_translates_every_set_of_the_run_once
  - src/use_cases/build_infobase.rs::only_the_markers_of_a_killed_creation_count_as_a_new_directory
  - src/use_cases/artifacts.rs::designer_loads_the_configuration_before_the_extension
  - src/use_cases/artifacts.rs::an_extension_after_another_configuration_set_reloads_the_main_one
  - src/use_cases/artifacts.rs::an_external_set_is_built_on_top_of_the_configuration
  - src/use_cases/artifacts.rs::an_ibcmd_walk_and_designer_share_the_build_infobase
  - src/use_cases/artifacts.rs::a_second_make_only_dumps_and_a_modified_module_loads_partially
  - src/use_cases/artifacts.rs::a_failed_dump_invalidates_the_build_base_for_the_next_make
  - src/use_cases/artifacts.rs::a_cancellation_during_creation_stops_before_load_and_leaves_no_ready_memory
  - tests/cli_make_download_all.rs::make_without_a_set_builds_every_package_in_one_build_infobase
  - tests/cli_make_download_all.rs::make_reuses_an_edt_build_infobase_until_forced
  - tests/cli_make_download_all.rs::make_reuses_the_build_infobase_when_the_package_is_inside_the_source_set
  - tests/cli_convert.rs::make_and_convert_share_the_build_infobase
---

# `make` и `convert --to package` используют одну базу сборки

Раннер хранит файловую базу и её память под `workPath/temp/build-infobase/`.
Каталог исключён из Git. Источники пакета — наборы проекта. Базу проекта команда
не выбирает. Конфигуратор собирает расширение поверх основной конфигурации и
публикует внешние обработки и отчёты из этой же базы.

Раннер пропускает перевод EDT при совпадении исходников и сохранённого XML. По
изменённому XML он решает, возможна ли частичная загрузка, тем же планировщиком,
что `push`, с фильтром файлов сборки. Без изменений сразу выгружает пакет.
При `make --full`, несовместимой
версии платформы, повреждении базы или отсутствии безопасного списка изменений
раннер создаёт свежую базу. Неудачная загрузка оставляет её память неготовой. Перевод EDT
повторяется при `make --full` для каждого набора прогона и при смене установки EDT. Если файл базы
изменил размер за успешную выгрузку, пакет публикуется, ответ предупреждает, а следующая сборка
идёт в свежей базе. Каталог, в котором прерванный запуск оставил только свои метки, считается
новым; прочий каталог без разметки команда не трогает и просит удалить.
Превью только проверяет цель и наличие инструментов.

Конфигуратор выполняет `/LoadConfigFromFiles`, при частичной загрузке с
`-partial -listFile`, затем `/DumpCfg`. Сборка не пишет файл версий в исходники
и не применяет конфигурацию к базе данных. Команды `ibcmd` описывает
`INV.USE-CASES.IBCMD-BUILDS-A-PACKAGE-IN-THE-BUILD-INFOBASE`.

**Reintroduction guard.** Владельцем базы и снимков остаётся `build_infobase::BuildSession`.
Проверки повторной сборки и перехода `make` → `convert` требуют пропуска импорта
в одной базе, а проверка отказа выгрузки требует свежей базы на следующем запуске.
