---
id: INV.USE-CASES.A-PUSH-WITHOUT-MEMORY-OF-THE-BASE-IS-REFUSED
check:
  - tests/cli_push_generation.rs::a_push_without_memory_of_the_base_is_refused_with_both_ways_out
  - tests/cli_push_generation.rs::a_push_force_loads_without_memory_and_remembers_the_generation
  - tests/cli_push_generation.rs::a_full_push_with_memory_of_another_base_is_refused_as_no_memory
  - tests/cli_push_generation.rs::a_preview_of_a_push_without_memory_names_the_refusal
  - tests/cli_push_generation.rs::a_base_created_by_the_runner_takes_the_first_push
  - tests/cli_test.rs::a_test_on_a_base_without_memory_is_refused_like_a_push
  - src/use_cases/exchange_guard.rs::a_base_created_by_the_runner_is_remembered
  - tests/cli_pull_memory.rs::first_full_pull_establishes_the_baseline_for_all_exporters
---

# Отправка в базу без памяти о ней отказывает

`push` в базу, о которой у рабочей копии нет памяти, отказывает родом `no_memory` до запуска
загрузки, в том числе когда база пуста: пустую базу раннер не различает, потому что значение
поколения у пустой базы зависит от версии платформы ([замер](../../../references/1c/confirmed-runtime-measurements.md)).
Отказ называет выходы: `pull`, если права база, — следующим шагом, и `push --force`, если
прав каталог. Выход под хранилищем конфигурации держит
`INV.USE-CASES.UNDER-A-REPOSITORY-A-REFUSAL-OFFERS-PULL-FORCE`.

Проверку проходит и полная загрузка `--full` (у MCP — `build_project` с `full_rebuild`);
обходит её только `push --force`. `test`, который грузит исходники перед прогоном, отказывает
так же, с теми же выходами, и его шаг сборки называет отказ. Превью `push` называет тот же
отказ, не запуская платформу.

Память о базе есть у каждого набора, который пойдёт в базу, отдельно: своя запись журнала
поколений или своя хеш-память этой пары «база ↔ каталог», в том числе пустая. Хеш-память,
записанная для другой пары, и хеш-память, которую не прочесть, памятью не считаются: они не
доказывают, что каталог происходит от этой базы.

Выходы уточняют свои правила: на общей базе — `INV.USE-CASES.A-SHARED-BASE-REFUSAL-OFFERS-PULL-FIRST-AND-NAMES-PUSH`,
у копии и у нового владельца до первой отправки —
`INV.USE-CASES.A-COPIED-BASE-OFFERS-NO-PULL-BEFORE-ITS-FIRST-PUSH` и
`INV.USE-CASES.A-NEW-OWNER-IS-OFFERED-NO-PULL-UNTIL-ITS-FIRST-PUSH`. Базы,
которой нет, отказ не касается: выходы называет
`INV.CLI.A-REFUSAL-WITHOUT-AN-OWN-BASE-NAMES-THE-WAYS-OUT`.

Память о базе пишут её создание раннером (`INV.CLI.INFOBASE-CREATE-FOLLOWS-THE-TARGET-KIND`,
`INV.CLI.INFOBASE-CREATE-FROM-COPIES-A-BASE`) — пустую хеш-память каждого набора — и полный
`pull` (`INV.USE-CASES.FULL-PULL-RECORDS-THE-PUBLISHED-TREE`), поэтому после них отказа нет.

Решение владельца от 06.10.2026: `--full` проходит проверки памяти и поколения, чужая и
нечитаемая память — отсутствие памяти, превью и `test` называют тот же отказ.

Источник: [`cli.html#refusals`](../../../docs/site/cli.html#refusals).
