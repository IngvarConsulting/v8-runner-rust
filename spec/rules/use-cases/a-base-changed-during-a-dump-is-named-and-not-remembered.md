---
id: INV.USE-CASES.A-BASE-CHANGED-DURING-A-DUMP-IS-NAMED-AND-NOT-REMEMBERED
check:
  - tests/cli_push_generation.rs::a_base_changed_during_a_pull_is_named_and_not_remembered
  - tests/cli_push_generation.rs::a_pull_records_the_generation_it_saw_before_and_after
---

# Базу, изменённую во время выгрузки, называют и не запоминают

`pull` спрашивает поколение до выгрузки и после неё тем же инструментом. Одно и то же —
поколение записывается в память с операцией `dump`. Разное — базу правили во время выгрузки:
ответ это называет, а память о поколении не обновляется, и следующая отправка снова видит
расхождение. Без ответа до или после запись не меняется. Выборка объектов поколения не
пишет: каталог с базой она не сводит.

Источник: [`sources.html#runner`](../../../docs/site/sources.html#runner).
