---
id: INV.USE-CASES.A-COPIED-BASE-OFFERS-NO-PULL-BEFORE-ITS-FIRST-PUSH
check:
  - tests/cli_push_generation.rs::a_copied_base_offers_no_pull_before_its_first_push
  - src/use_cases/exchange_guard.rs::a_copy_mark_is_memory_and_offers_no_pull
---

# До первой отправки в копию базы выгрузку не предлагают

До первой удачной отправки в базу, созданную `infobase create --from`, ни один ответ не
предлагает `pull`: отказ называет `push --force` и то, что база — копия другой базы. Признак копии, который не прочесть, стоит.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).
