---
id: INV.USE-CASES.A-REPLACEMENT-REFUSAL-NAMES-THE-WAYS-OUT
check:
  - src/use_cases/destruction_guard.rs::the_refusal_names_the_ways_out_the_caller_has
  - src/use_cases/dump_config.rs::a_caller_without_a_force_way_out_is_not_told_to_force
  - tests/cli_bootstrap.rs::a_clone_refusal_does_not_offer_force
---

# Отказ сторожа замены называет выходы, которые есть у вызывающего

Отказ заменить каталог с невосстановимой работой называет выход «сохранить работу в
системе контроля версий (закоммитить или спрятать) и повторить» всегда. Замену каталога с
потерей работы он называет только вызывающему, у которого она есть: `pull` и MCP
`dump_config` — через `pull <SET> --force`, `convert` — через тот же вызов с `--force`.
Вызывающему без замены (`clone`) отказ оставляет только сохранение работы.
