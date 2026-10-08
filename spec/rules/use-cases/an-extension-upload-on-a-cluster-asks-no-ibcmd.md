---
id: INV.USE-CASES.AN-EXTENSION-UPLOAD-ON-A-CLUSTER-ASKS-NO-IBCMD
check: [src/use_cases/load_artifact.rs::an_extension_upload_on_a_cluster_never_runs_ibcmd]
---

# `upload .cfe` у кластера не спрашивает `ibcmd`

Перед загрузкой `.cfe` в кластерную базу список установленных расширений спрашивает тот же
исполнитель, что и `extensions` у кластера (агент), а не `ibcmd` с серверным подключением к
базе: `upload .cfe` на кластере `ibcmd` не вызывает и секции `dbms` не требует.
