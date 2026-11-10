---
id: INV.PLATFORM.A-MANAGED-SERVER-KEEPS-ITS-OWN-DATA-DIRECTORY
check:
  - src/use_cases/managed_stand/standalone.rs::a_dbms_server_starts_over_its_database_with_the_password_on_stdin
  - src/platform/service_record.rs::a_stand_service_lives_under_the_local_path
---

# У поднятого раннером `ibsrv` свой каталог `--data`

Каталог `--data` лежит под `<localPath>/infobases/<база>/managed/standalone/` и не совпадает с `ibcmd-data` под
`workPath`, который раннер отдаёт разовым запускам `ibcmd`. Данные остальных служб стенда — `postgres`, `cluster`, `ras` —
лежат рядом, под тем же `managed/`.
