---
id: INV.USE-CASES.CONVERT-SEQUENCES-ARE-MEASURED-ON-A-LIVE-PLATFORM
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/416
---

# Последовательности `convert` с пакетом замерены на живой платформе

Разбор пакета `convert` (`INV.USE-CASES.IBCMD-EXPORTS-A-PACKAGE-IN-A-THROWAWAY-BASE`)
проходит на живой платформе в том виде, в каком его вызывает раннер: `ibcmd infobase
create --data <d> --db-path <ib>`, затем `ibcmd config --data <d> --db-path <ib> export
--file=<пакет> <каталог>` в пустой свежей базе, для `.cf` и для `.cfe`, и XML совпадает по
содержимому с выгрузкой Конфигуратора. Сборку пакета с `--data` держит
`INV.USE-CASES.MAKE-SEQUENCES-ARE-MEASURED-ON-A-LIVE-PLATFORM`.
