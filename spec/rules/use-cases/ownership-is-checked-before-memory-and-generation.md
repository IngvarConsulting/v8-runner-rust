---
id: INV.USE-CASES.OWNERSHIP-IS-CHECKED-BEFORE-MEMORY-AND-GENERATION
check:
  - tests/cli_infobase_owner.rs::ownership_is_refused_before_foreign_memory
  - src/use_cases/transport.rs::a_base_of_another_copy_stops_the_dispatch_before_the_scenario
  - tests/architecture_guardrails.rs::the_owner_of_a_file_base_is_checked_in_one_place
  - tests/cli_push_generation.rs::a_push_without_memory_of_the_base_is_refused_with_both_ways_out
---

# Сначала владелец, затем память, затем поколение

Проверки перед обменом с базой идут по порядку: чья база, есть ли о ней память, не ушла ли
она вперёд. Отказ называет первую непройденную: без памяти о базе поколение не спрашивается и
платформа не запускается.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).
