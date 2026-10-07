---
id: INV.CLI.CONVERT-WORKS-ON-PROJECT-SOURCE-SETS
check:
  - tests/cli_convert.rs::convert_without_source_set_processes_all_source_sets_into_work_path_out
  - tests/cli_convert.rs::convert_without_a_set_to_a_package_takes_the_configuration_packages
  - tests/cli_convert.rs::convert_an_external_set_to_a_package_is_refused
  - tests/cli_convert.rs::convert_unknown_source_set_json_keeps_convert_command_identity_before_workspace_lock
  - tests/cli_convert.rs::convert_output_root_rejects_source_overlap_before_workspace_lock
  - tests/cli_convert.rs::convert_a_package_file_to_xml_exports_it_in_a_throwaway_base
  - tests/cli_convert.rs::convert_needs_no_infobase_and_refuses_the_infobase_key
  - tests/cli_global_flags.rs::a_leaf_that_selects_no_base_refuses_the_base_key
---

# `convert` работает над наборами проекта

Без указания набора обрабатываются все наборы в порядке настроек, с указанием — один
названный; неизвестное имя даёт отказ до замка. `--to package` без набора берёт пакеты
конфигурации проекта — основную конфигурацию и расширения порядком обхода, — а набор
внешних файлов, названный с `--to package`, — отказ валидации.

Информационная база команде не нужна: проект без базы и без местного слоя переводит, а
`--infobase` у `convert` — отказ валидации.

`--output` задаёт только корень результата, а у файла пакета — сам каталог XML, и
проверяется на пересечение с исходниками, базовым и рабочим каталогами.

Как называется набор, держит `INV.CLI.CONVERT-TAKES-A-SET-OR-A-PACKAGE-FILE`; направление —
`INV.CLI.CONVERT-DIRECTION-IS-SET-BY-TO`; исполнителей для пакета —
`INV.CLI.A-PACKAGE-DIRECTION-OF-CONVERT-HAS-AN-EXECUTOR-CHAIN`.
