---
id: INV.USE-CASES.MAKE-SEQUENCES-ARE-MEASURED-ON-A-LIVE-PLATFORM
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/416
---

# Последовательности `make` замерены на живой платформе

Сборка `make` во временной базе
(`INV.USE-CASES.MAKE-BUILDS-PACKAGES-FROM-SOURCES-IN-A-THROWAWAY-BASE`) проходит на живой
платформе в том виде, в каком её вызывает раннер:

- Конфигуратор на свежей базе `CREATEINFOBASE`, к которой он подключается через
  `/IBConnectionString`: загрузка без файла версий и выгрузка `/DumpCfg`;
- `ibcmd` с `--data` своего каталога и `config import --out`;
- внешние обработки и отчёты в базе, куда основная конфигурация загружена, но не применена.
