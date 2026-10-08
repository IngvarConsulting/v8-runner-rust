---
id: INV.USE-CASES.A-PUSH-WITHOUT-MEMORY-OF-THE-BASE-IS-REFUSED
check:
  - tests/cli_push_generation.rs::a_push_without_memory_of_the_base_is_refused_with_both_ways_out
  - tests/cli_push_generation.rs::a_push_force_loads_without_memory_and_remembers_the_generation
  - tests/cli_push_generation.rs::a_full_push_with_memory_of_another_base_is_refused_as_no_memory
---

# Отправка в базу без памяти о ней отказывает

`push` в базу, о которой у рабочей копии нет памяти, отказывает родом `no_memory` до запуска
загрузки, в том числе когда база пуста: пустую базу раннер не различает, потому что значение
поколения у пустой базы зависит от версии платформы ([замер](../../../references/1c/confirmed-runtime-measurements.md)).
Отказ называет выходы: выгрузку, если права база, — следующим шагом, и `push --force`, если
прав каталог. Проверку проходит и полная загрузка `--full` (у MCP — `build_project` с
`full_rebuild`); обходит её только `push --force`.

Что считается памятью, держит `INV.USE-CASES.WHAT-COUNTS-AS-MEMORY-OF-THE-BASE`. Выходы
уточняют свои правила: без ответа о поколении —
`INV.USE-CASES.WITHOUT-A-GENERATION-ANSWER-A-NO-MEMORY-REFUSAL-OFFERS-PULL-FORCE`, под
хранилищем — `INV.USE-CASES.UNDER-A-REPOSITORY-A-REFUSAL-OFFERS-PULL-FORCE`, у копии и у нового
владельца до первой отправки — `INV.USE-CASES.A-COPIED-BASE-OFFERS-NO-PULL-BEFORE-ITS-FIRST-PUSH`
и `INV.USE-CASES.A-NEW-OWNER-IS-OFFERED-NO-PULL-UNTIL-ITS-FIRST-PUSH`. Базы, которой нет,
отказ не касается: выходы называет `INV.CLI.A-REFUSAL-WITHOUT-AN-OWN-BASE-NAMES-THE-WAYS-OUT`.

Решение владельца от 06.10.2026: `--full` проходит проверки памяти и поколения.

Источник: [`cli.html#refusals`](../../../docs/site/cli.html#refusals).
