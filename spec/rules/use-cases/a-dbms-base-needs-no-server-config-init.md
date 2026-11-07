---
id: INV.USE-CASES.A-DBMS-BASE-NEEDS-NO-SERVER-CONFIG-INIT
check:
  - src/use_cases/managed_stand/standalone.rs::a_dbms_server_starts_over_its_database_with_the_password_on_stdin
---

# Для базы в СУБД `server config init` не вызывается

`ibcmd server config init` пишет пароль СУБД в файл открытым текстом, а `ibsrv` работает и без него, поэтому раннер не
вызывает его для базы в СУБД.
