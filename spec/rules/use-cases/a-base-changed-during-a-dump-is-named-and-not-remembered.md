---
id: INV.USE-CASES.A-BASE-CHANGED-DURING-A-DUMP-IS-NAMED-AND-NOT-REMEMBERED
check:
  - tests/cli_push_generation.rs::a_base_changed_during_a_pull_is_named_and_not_remembered
  - tests/cli_push_generation.rs::a_first_pull_that_saw_the_base_change_leaves_the_next_push_refused
  - tests/cli_push_generation.rs::a_pull_records_the_generation_it_saw_before_and_after
  - tests/cli_push_generation.rs::a_pull_without_an_answer_or_of_objects_leaves_the_record_as_it_was
---

# Базу, изменённую во время выгрузки, называют и не запоминают

`pull` спрашивает поколение до выгрузки и после неё тем же инструментом. Одно и то же —
поколение записывается в память с операцией `dump`. Разное — базу правили во время выгрузки:
ответ это называет, а в память ложится поколение, которое было до выгрузки, и следующая
отправка видит расхождение и отказывает `non_fast_forward` — в том числе после первой
выгрузки, когда памяти о базе до неё не было. Без ответа до или после запись не меняется.
Выборка объектов поколения не пишет: каталог с базой она не сводит.

Источник: [`sources.html#runner`](../../../docs/site/sources.html#runner).
