---
id: INV.USE-CASES.CONVERT-SEQUENCES-ARE-MEASURED-ON-A-LIVE-PLATFORM
check:
  - src/platform/ibcmd.rs::config_export_file_uses_config_mode_and_the_target_context
  - tests/cli_convert.rs::convert_a_package_file_to_xml_exports_it_in_a_throwaway_base
---

# Последовательности `convert` с пакетом замерены на живой платформе

Разбор пакета `convert` (`INV.USE-CASES.IBCMD-EXPORTS-A-PACKAGE-IN-A-THROWAWAY-BASE`)
проходит на живой платформе в том виде, в каком его вызывает раннер: `ibcmd infobase
create --data <d> --db-path <ib>`, затем `ibcmd config --data <d> --db-path <ib> export
--file=<пакет> <каталог>` в пустой свежей базе, для `.cf` и для `.cfe`, и XML совпадает по
содержимому с выгрузкой Конфигуратора. Сборку пакета с `--data` держит
`INV.USE-CASES.MAKE-SEQUENCES-ARE-MEASURED-ON-A-LIVE-PLATFORM`.

Замер — [`confirmed-runtime-measurements.md`](../../../references/1c/confirmed-runtime-measurements.md), раздел «Разбор пакета `convert` во временной базе». Проверки держат командные строки в замеренной форме.
