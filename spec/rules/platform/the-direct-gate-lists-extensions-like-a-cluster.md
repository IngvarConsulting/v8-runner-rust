---
id: INV.PLATFORM.THE-DIRECT-GATE-LISTS-EXTENSIONS-LIKE-A-CLUSTER
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/184
---

# Состав расширений за прямым шлюзом читается как у кластера

`pull --all` и `download` без набора спрашивают состав расширений автономного сервера,
когда исполнитель — Конфигуратор по прямому шлюзу, вызовом `/DumpDBCfgList -AllExtensions`
и разбирают ответ так же, как у базы в кластере. Ответ этой команды за прямым шлюзом не
замерен.
