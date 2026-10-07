---
id: INV.USE-CASES.IBCMD-BUILDS-A-PACKAGE-IN-A-THROWAWAY-BASE
check:
  - tests/cli_convert.rs::convert_a_set_to_a_package_builds_it_with_ibcmd_in_a_throwaway_base
  - tests/cli_convert.rs::convert_without_a_set_to_a_package_takes_the_configuration_packages
  - tests/cli_convert.rs::convert_an_edt_set_to_a_package_goes_through_xml_in_the_throwaway_base
  - src/platform/ibcmd.rs::config_import_to_file_always_passes_out
---

# `convert` в пакет через `ibcmd` идёт во временной базе раннера

Без существующей базы `ibcmd config import --out` не работает, а без `--out` та же
команда загружает исходники в базу ([замер](../../../references/1c/confirmed-runtime-measurements.md)). Поэтому `convert` в пакет,
выполняемый `ibcmd`, собирает его во временной базе раннера тем же владельцем, что `make`
(`INV.USE-CASES.MAKE-BUILDS-PACKAGES-FROM-SOURCES-IN-A-THROWAWAY-BASE`): база под
`workPath` со своим каталогом данных `--data`, импорт всегда с `--out`, база убирается после
прогона. Прогону без набора служит одна база на все пакеты. Исходники формата EDT сперва
переводит в XML `1cedtcli` в каталог этой базы. База проекта в этом не участвует, и для
пользователя сборка остаётся сборкой без базы.
