---
id: INV.PLATFORM.THE-DIRECT-GATE-LISTS-EXTENSIONS-LIKE-A-CLUSTER
check:
  - tests/cli_standalone_direct_gate.rs::pull_and_download_discover_extensions_through_the_direct_gate
---

# Состав расширений за прямым шлюзом читается как у кластера

`pull --all` и `download` без набора спрашивают состав расширений автономного сервера,
когда исполнитель — Конфигуратор по прямому шлюзу, вызовом `/DumpDBCfgList -AllExtensions`
и разбирают ответ так же, как у базы в кластере.
[Замер #184](https://github.com/IngvarConsulting/v8-runner-rust/pull/751)
