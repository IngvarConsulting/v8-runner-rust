---
id: INV.CLI.CONVERT-DIRECTION-IS-SET-BY-TO
check:
  - src/cli/args.rs::convert_to_takes_xml_edt_or_package
  - tests/cli_convert.rs::convert_without_source_set_processes_all_source_sets_into_work_path_out
  - tests/cli_convert.rs::convert_single_source_set_uses_inferred_edt_to_designer_direction
  - tests/cli_convert.rs::convert_a_set_to_a_package_builds_it_with_ibcmd_in_a_throwaway_base
  - tests/cli_convert.rs::convert_an_edt_set_to_a_package_goes_through_xml_in_the_throwaway_base
  - tests/cli_convert.rs::convert_a_package_file_to_xml_exports_it_in_a_throwaway_base
  - tests/cli_convert.rs::convert_a_package_file_without_to_goes_to_xml_under_work_path
  - tests/cli_convert.rs::convert_refuses_a_direction_that_is_not_a_conversion
  - tests/cli_convert.rs::convert_to_names_the_default_direction_explicitly
---

# Направление `convert` задаёт `--to`

`--to xml|edt|package` задаёт, во что переводить: XML платформы, проект EDT или пакет.
Ответ называет направление в `data.direction`.

| Вход | Без `--to` | `--to xml` | `--to edt` | `--to package` |
| --- | --- | --- | --- | --- |
| наборы формата Конфигуратора | `DESIGNER_TO_EDT` | отказ | `DESIGNER_TO_EDT` | `DESIGNER_TO_PACKAGE` |
| наборы формата EDT | `EDT_TO_DESIGNER` | `EDT_TO_DESIGNER` | отказ | `EDT_TO_PACKAGE` |
| файл пакета | `PACKAGE_TO_DESIGNER` | `PACKAGE_TO_DESIGNER` | отказ | отказ |

Отказ — род `validation` до замка и до платформы; текст называет допустимые значения
`--to`. `EDT_TO_PACKAGE` сперва переводит наборы в XML `1cedtcli`, затем собирает пакет.
