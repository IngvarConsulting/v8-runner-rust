---
id: INV.CLI.SERVICE-COMMANDS-TAKE-A-DECLARED-BASE
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/536
---

# Команды служб принимают только объявленную базу

`--infobase` у `dbms`, `cluster` и `standalone` называет объявленную базу, а строка соединения — отказ: у неё нет записи и
нет секции `managed`.
