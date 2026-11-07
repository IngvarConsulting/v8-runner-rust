---
id: INV.PLATFORM.A-MANAGED-SERVER-KEEPS-ITS-OWN-DATA-DIRECTORY
check:
  - src/use_cases/managed_stand/standalone.rs::a_dbms_server_starts_over_its_database_with_the_password_on_stdin
---

# У поднятого раннером `ibsrv` свой каталог `--data`

Каталог `--data` лежит под `managed/` и не совпадает с `ibcmd-data`, который раннер отдаёт разовым запускам `ibcmd`.
