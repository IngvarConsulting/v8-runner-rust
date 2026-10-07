---
id: INV.USE-CASES.WHAT-COUNTS-AS-MEMORY-OF-THE-BASE
check:
  - tests/cli_push_generation.rs::a_generation_record_does_not_make_memory_of_another_pair_own
  - tests/cli_push_generation.rs::a_full_push_with_memory_of_another_base_is_refused_as_no_memory
  - tests/cli_push_generation.rs::a_base_created_by_the_runner_takes_the_first_push
  - src/use_cases/exchange_guard.rs::a_base_created_by_the_runner_is_remembered
  - src/use_cases/exchange_guard.rs::a_created_base_remembers_the_set_it_was_assembled_from
  - tests/cli_pull_memory.rs::first_full_pull_establishes_the_baseline_for_all_exporters
---

# Что считается памятью о базе

Память о базе есть у каждого набора, который пойдёт в базу, отдельно: своя хеш-память этой
пары «база ↔ каталог», в том числе пустая, или, когда хеш-памяти нет, своя запись журнала
поколений. Хеш-память, записанная для другой пары, и хеш-память, которую не прочесть,
памятью не считаются, даже рядом со своей записью поколения: они не доказывают, что каталог
происходит от этой базы.

Память пишут создание базы раннером — у набора, из которого база собрана, его дерево, у
каждого другого набора пустую хеш-память (`INV.CLI.INFOBASE-CREATE-FOLLOWS-THE-TARGET-KIND`,
`INV.CLI.INFOBASE-CREATE-FROM-COPIES-A-BASE`), —
полный `pull` (`INV.USE-CASES.FULL-PULL-RECORDS-THE-PUBLISHED-TREE`), выгрузка, о которой
инструмент ответил поколением, и удачный `push`.

Решение владельца от 06.10.2026: чужая и нечитаемая память — отсутствие памяти.
