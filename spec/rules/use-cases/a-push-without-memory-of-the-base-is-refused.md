---
id: INV.USE-CASES.A-PUSH-WITHOUT-MEMORY-OF-THE-BASE-IS-REFUSED
check:
  - tests/cli_push_generation.rs::a_push_without_memory_of_the_base_is_refused_with_both_ways_out
  - tests/cli_push_generation.rs::a_push_force_loads_without_memory_and_remembers_the_generation
  - src/use_cases/exchange_guard.rs::a_base_created_by_the_runner_is_remembered
  - tests/cli_pull_memory.rs::first_full_pull_establishes_the_baseline_for_all_exporters
---

# Отправка в базу без памяти о ней отказывает

`push` в базу, о которой у рабочей копии нет памяти, отказывает до запуска загрузки, в том
числе когда база пуста: пустую базу раннер не различает, потому что значение поколения у
пустой базы зависит от версии платформы ([замер](../../../references/1c/confirmed-runtime-measurements.md)). Отказ называет выходы: `pull`, если
права база, — следующим шагом, и `push --force`, если прав каталог. Выход под хранилищем
конфигурации держит `INV.USE-CASES.UNDER-A-REPOSITORY-A-REFUSAL-OFFERS-PULL-FORCE`.

Память о базе есть у каждого набора, который пойдёт в базу, отдельно: запись журнала
поколений или непустая хеш-память этой пары «база ↔ каталог». Базу, адреса которой раннер не
распознаёт, он не помнит никогда: отказ называет ей один выход — `push --force`. Превью
памяти не требует.

Выходы уточняют свои правила: на общей базе — `INV.USE-CASES.A-SHARED-BASE-REFUSAL-OFFERS-PULL-FIRST-AND-NAMES-PUSH`
у копии и у нового владельца до первой отправки —
`INV.USE-CASES.A-COPIED-BASE-OFFERS-NO-PULL-BEFORE-ITS-FIRST-PUSH` и
`INV.USE-CASES.A-NEW-OWNER-IS-OFFERED-NO-PULL-UNTIL-ITS-FIRST-PUSH`. Памяти, записанной для
другой базы, отвечает `INV.USE-CASES.FOREIGN-MEMORY-IS-NOT-USED` со своими выходами. Базы,
которой нет, отказ не касается: выходы называет
`INV.CLI.A-REFUSAL-WITHOUT-AN-OWN-BASE-NAMES-THE-WAYS-OUT`.

Память о базе пишут её создание раннером (`INV.CLI.INFOBASE-CREATE-FOLLOWS-THE-TARGET-KIND`,
`INV.CLI.INFOBASE-CREATE-FROM-COPIES-A-BASE`) и полный `pull`
(`INV.USE-CASES.FULL-PULL-RECORDS-THE-PUBLISHED-TREE`), поэтому после них отказа нет.

Источник: [`cli.html#refusals`](../../../docs/site/cli.html#refusals).
