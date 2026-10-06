---
id: INV.CLI.INIT-FORCE-MOVES-THE-PROJECT-INFOBASE-SYNONYM-INTO-THE-LOCAL-LAYER
check:
  - src/use_cases/config_init.rs::force_carries_the_infobase_synonym_of_the_project_file_into_the_local_layer
---

# `init --force` переносит проектный `infobase:` в местный слой

`init --force` переписывает проектный файл без прежнего ключа `infobase:`, а слитую
секцию `origin` переносит в местный слой, так что действующий `origin` после команды тот
же, что до неё, если `--infobase` не назвал другой адрес.

Источник: [`cli.html#map`](../../../docs/site/cli.html#map).
