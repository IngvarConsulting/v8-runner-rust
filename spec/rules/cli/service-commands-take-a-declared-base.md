---
id: INV.CLI.SERVICE-COMMANDS-TAKE-A-DECLARED-BASE
check:
  - src/use_cases/managed_stand/mod.rs::an_ad_hoc_connection_is_refused
  - tests/cli_dbms.rs::a_connection_string_is_refused
---

# Команды служб принимают только объявленную базу

`--infobase` у `dbms`, `cluster` и `standalone` называет объявленную базу, а строка соединения — отказ: у неё нет записи и
нет секции `managed`.
