---
id: INV.USE-CASES.MAKE-SEQUENCES-ARE-MEASURED-ON-A-LIVE-PLATFORM
check:
  - src/use_cases/artifacts.rs::designer_builds_a_cf_from_the_sources_in_a_throwaway_base
  - src/use_cases/artifacts.rs::ibcmd_builds_with_out_and_its_own_data_directory
  - src/use_cases/artifacts.rs::an_external_set_is_built_on_top_of_the_configuration
---

# Последовательности `make` замерены на живой платформе

Сборка `make` во временной базе
(`INV.USE-CASES.MAKE-BUILDS-PACKAGES-FROM-SOURCES-IN-A-THROWAWAY-BASE`) проходит на живой
платформе в том виде, в каком её вызывает раннер:

- Конфигуратор на свежей базе `CREATEINFOBASE`, к которой он подключается через
  `/IBConnectionString`: загрузка без файла версий и выгрузка `/DumpCfg`;
- `ibcmd` с `--data` своего каталога и `config import --out`;
- внешние обработки и отчёты в базе, куда основная конфигурация загружена, но не применена.

Замер — [`confirmed-runtime-measurements.md`](../../../references/1c/confirmed-runtime-measurements.md), раздел «Сборка `make` во временной базе». Проверки держат командные строки в замеренной форме.
