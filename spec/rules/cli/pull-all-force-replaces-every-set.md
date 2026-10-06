---
id: INV.CLI.PULL-ALL-FORCE-REPLACES-EVERY-SET
check:
  - tests/cli_pull_all.rs::pull_all_force_replaces_every_set
---

# `pull --all --force` заменяет каждый набор

С `--force` каждый набор обхода выгружается как `pull <SET> --force`: каталог заменяется
состоянием базы, а уничтоженное называется в `losses` его выгрузки. Объявляемые наборы
выгружаются так же и объявляются.
