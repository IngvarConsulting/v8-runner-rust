---
id: INV.PLATFORM.THE-DIRECT-GATE-LISTS-EXTENSIONS-LIKE-A-CLUSTER
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/184
---

# Состав расширений за прямым шлюзом читается как у кластера

`pull --all` и `download` без набора спрашивают состав расширений автономного сервера,
когда исполнитель — Конфигуратор по прямому шлюзу, вызовом `/DumpDBCfgList -AllExtensions`
и разбирают ответ так же, как у базы в кластере. Ответ этой команды за прямым шлюзом не
замерен. Выгрузку одного файла версий (`-configDumpInfoOnly`) Конфигуратор по прямому шлюзу
не получает: ответа о поколении у него нет
(`INV.USE-CASES.A-CLUSTER-DESIGNER-GENERATION-IS-READ-LIKE-A-FILE-ONE`), а без него совпадение
каталога и базы не доказано.
