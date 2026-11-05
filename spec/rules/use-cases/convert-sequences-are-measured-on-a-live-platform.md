---
id: INV.USE-CASES.CONVERT-SEQUENCES-ARE-MEASURED-ON-A-LIVE-PLATFORM
check:
  - src/platform/ibcmd.rs::config_export_file_uses_config_mode_and_the_target_context
  - tests/cli_convert.rs::convert_a_package_file_to_xml_exports_it_in_a_throwaway_base
---

# Последовательности `convert` с пакетом замерены на живой платформе

Для разбора пакета `convert` (`INV.USE-CASES.IBCMD-EXPORTS-A-PACKAGE-IN-A-THROWAWAY-BASE`)
раннер создаёт пустую базу командой `ibcmd infobase --data <d> --db-path <ib> create`, затем
разбирает пакет командой `ibcmd config --data <d> --db-path <ib> export --threads=1
--file=<пакет> <каталог>`. Проверки держат аргументы в форме, замеренной для `.cf` в #471
и #476. В исходном замере #416 XML из `.cf` и `.cfe` совпал по содержимому с выгрузкой
Конфигуратора.
Замеры приведены в [`confirmed-runtime-measurements.md`](../../../references/1c/confirmed-runtime-measurements.md)
в разделах «Разбор пакета `convert` во временной базе» и «Инкрементальная сборка в постоянной
базе сборки». Сборку пакета с `--data` держит
`INV.USE-CASES.MAKE-SEQUENCES-ARE-MEASURED-ON-A-LIVE-PLATFORM`.
