---
id: INV.USE-CASES.A-COPIED-BASE-OFFERS-NO-PULL-BEFORE-ITS-FIRST-PUSH
check:
  - tests/cli_push_generation.rs::a_copied_base_offers_no_pull_before_its_first_push
  - src/use_cases/exchange_guard.rs::a_copy_mark_is_memory_and_offers_no_pull
  - tests/cli_infobase_copy.rs::a_push_of_one_set_keeps_the_copy_mark_until_every_set_is_remembered
---

# До первой отправки в копию базы выгрузку не предлагают

Пока стоит признак копии базы, созданной `infobase create --from`, — до удачной отправки,
после которой у каждого набора снова есть своя память, — ни один ответ не предлагает `pull`: отказ называет `push --force` и то, что база — копия другой базы. Признак копии, который не прочесть, стоит.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).
