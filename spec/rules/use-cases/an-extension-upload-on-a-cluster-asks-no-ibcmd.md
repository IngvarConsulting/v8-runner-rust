---
id: INV.USE-CASES.AN-EXTENSION-UPLOAD-ON-A-CLUSTER-ASKS-NO-IBCMD
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/431
---

# `upload .cfe` у кластера не спрашивает `ibcmd`

Перед загрузкой `.cfe` в кластерную базу список установленных расширений спрашивает тот же
исполнитель, что и `extensions` у кластера (агент), а не `ibcmd` с серверным подключением к
базе: `upload .cfe` на кластере `ibcmd` не вызывает и секции `dbms` не требует.
