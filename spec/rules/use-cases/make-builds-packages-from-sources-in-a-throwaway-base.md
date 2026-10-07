---
id: INV.USE-CASES.MAKE-BUILDS-PACKAGES-FROM-SOURCES-IN-A-THROWAWAY-BASE
check:
  - src/use_cases/artifacts.rs::designer_builds_a_cf_from_the_sources_in_a_throwaway_base
  - src/use_cases/artifacts.rs::designer_loads_the_configuration_before_the_extension
  - src/use_cases/artifacts.rs::ibcmd_builds_with_out_and_its_own_data_directory
  - src/use_cases/artifacts.rs::edt_sources_are_converted_to_xml_inside_the_throwaway_base_first
  - src/use_cases/artifacts.rs::an_external_set_is_built_on_top_of_the_configuration
  - src/use_cases/artifacts.rs::an_ibcmd_walk_gives_externals_a_designer_base_of_their_own
  - src/use_cases/artifacts.rs::an_edt_set_is_converted_once_per_run
  - src/platform/ibcmd.rs::config_import_to_file_always_passes_out
  - tests/cli_make_download_all.rs::make_without_a_set_builds_every_package_in_one_throwaway_base
---

# `make` собирает пакет из исходников во временной базе раннера

`make` собирает `.cf` и `.cfe` из исходников набора, а не выгружает базу проекта: базу
проекта он не открывает. Пакет собирается во временной файловой базе раннера под
`workPath`, а не в системном временном каталоге.

- `ibcmd` создаёт её `infobase create` со своим каталогом данных `--data` внутри неё —
  общий `workPath/ibcmd-data` не используется — и собирает пакет `config import` всегда с
  `--out`: без него та же команда загрузила бы исходники в базу
  ([замер](../../../references/1c/confirmed-runtime-measurements.md)). Основная конфигурация
  расширению не нужна.
- Конфигуратор создаёт её `CREATEINFOBASE`, загружает исходники `/LoadConfigFromFiles` без
  `-updateConfigDumpInfo` и без `/UpdateDBCfg` и выгружает пакет `/DumpCfg`. Расширение он
  загружает с `-Extension` поверх основной конфигурации, которую база получает первой.
- Внешние обработки и отчёты собирает всегда Конфигуратор в базе, которую создал он сам:
  сперва основная конфигурация проекта тем же `/LoadConfigFromFiles`, затем
  `/LoadExternalDataProcessorOrReportFromFiles`. Базу, созданную `ibcmd`, Конфигуратор не
  открывает: в обходе `ibcmd` у внешних наборов своя база.
- Исходники формата EDT сперва переводит в XML `1cedtcli` — шагом сборки `push` — в
  каталог временной базы, один раз за прогон: другая база прогона берёт тот же перевод.

Эти последовательности проверены на поддельной платформе; живой замер —
`INV.USE-CASES.MAKE-SEQUENCES-ARE-MEASURED-ON-A-LIVE-PLATFORM`.
