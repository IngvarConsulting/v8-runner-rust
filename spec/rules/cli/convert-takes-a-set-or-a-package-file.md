---
id: INV.CLI.CONVERT-TAKES-A-SET-OR-A-PACKAGE-FILE
check:
  - src/use_cases/request.rs::a_cf_or_cfe_argument_names_a_package_file_and_anything_else_a_set
  - src/cli/args.rs::parses_convert_with_source_set
  - tests/cli_help.rs::convert_help_uses_output_target_root_name
  - tests/cli_convert.rs::convert_a_package_file_to_xml_exports_it_in_a_throwaway_base
---

# Позиционный аргумент `convert` — набор или файл пакета

Аргумент с расширением `.cf` или `.cfe`, в любом регистре, считается файлом пакета,
остальное — именем набора исходников проекта. Скрытый ключ `--source-set <имя>` — прежнее
написание позиционного набора: справка его не показывает, и он называет набор при любом
имени, даже оканчивающемся на `.cf`.
