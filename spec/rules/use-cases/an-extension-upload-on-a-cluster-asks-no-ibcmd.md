---
id: INV.USE-CASES.AN-EXTENSION-UPLOAD-ON-A-CLUSTER-ASKS-NO-IBCMD
check:
  - src/use_cases/load_artifact.rs::an_extension_upload_on_a_cluster_never_runs_ibcmd
  - tests/cli_extensions.rs::cluster_designer_names_are_used_by_status_and_extension_upload
---

# `upload .cfe` у кластера не спрашивает `ibcmd`

Перед загрузкой `.cfe` в кластерную базу список установленных расширений спрашивает тот же
исполнитель, что и `extensions list` у кластера. Для пакетного Конфигуратора достаточно имён:
в списке есть и загруженное, ещё не применённое расширение. Кластер и автономный сервер не
требуют `ibcmd` и секции `dbms`. Если выбранный агент не поднялся, загрузка отказывает с родом
`environment`. Если исполнитель получил вопрос и не ответил, наличие остаётся неизвестным,
как у `ibcmd` файловой базы.
