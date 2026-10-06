---
id: INV.CLI.PULL-ALL-STOPS-AT-THE-FIRST-REFUSED-SET
check:
  - tests/cli_pull_all.rs::a_refused_set_stops_the_walk_before_anything_is_declared
---

# Отказ набора останавливает обход

Отказ при выгрузке набора останавливает `pull --all`: наборы после него не выгружаются и не
объявляются, ответ несёт выгруженное до отказа и отказавший набор, а совет отказа — тот же,
что у `pull <SET>`.
