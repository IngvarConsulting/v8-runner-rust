---
id: INV.USE-CASES.A-DBMS-BASE-NEEDS-NO-SERVER-CONFIG-INIT
check:
  - src/use_cases/managed_stand/standalone.rs::a_database_is_created_by_ibcmd_without_a_server_config
---

# Для базы в СУБД `server config init` не вызывается

`ibcmd server config init` пишет пароль СУБД в файл открытым текстом, а `ibsrv` работает и без него, поэтому раннер не
вызывает его для базы в СУБД.
