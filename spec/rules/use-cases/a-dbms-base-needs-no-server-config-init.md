---
id: INV.USE-CASES.A-DBMS-BASE-NEEDS-NO-SERVER-CONFIG-INIT
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/537
---

# Для базы в СУБД `server config init` не вызывается

`ibcmd server config init` пишет пароль СУБД в файл открытым текстом, а `ibsrv` работает и без него, поэтому раннер не
вызывает его для базы в СУБД.
