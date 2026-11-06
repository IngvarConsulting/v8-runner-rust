---
id: INV.USE-CASES.IBCMD-BUILDS-A-PACKAGE-IN-THE-BUILD-INFOBASE
check:
  - src/use_cases/artifacts.rs::ibcmd_imports_and_saves_the_working_configuration_with_its_own_data_directory
  - tests/cli_convert.rs::convert_a_set_to_a_package_builds_it_with_ibcmd_in_the_build_infobase
  - tests/cli_convert.rs::convert_an_edt_set_to_a_package_goes_through_xml_in_the_build_infobase
  - tests/cli_convert.rs::convert_loads_a_changed_module_with_ibcmd_files_partial
---

# `ibcmd` собирает пакет в общей базе сборки

У `ibcmd` свой `--data` внутри базы сборки. Полный импорт загружает XML в
рабочую конфигурацию. Частичный импорт выполняет `config import files --partial
--base-dir` со списком путей относительно корня XML. Затем `config save`
выгружает рабочую конфигурацию. Для загрузки команда не передаёт `--out`, для
выгрузки — `--db`. Замер #471 подтвердил эти команды, в том числе при смене
исполнителя той же версии платформы.
