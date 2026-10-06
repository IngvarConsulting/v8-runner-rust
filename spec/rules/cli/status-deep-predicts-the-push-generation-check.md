---
id: INV.CLI.STATUS-DEEP-PREDICTS-THE-PUSH-GENERATION-CHECK
check:
  - tests/cli_status.rs::status_deep_predicts_the_push_generation_check
---

# `status --deep` сверяет поколение так, как сверит его `push`

`status --deep` спрашивает поколение каждого набора тем исполнителем, которого `push` выбрал
бы для этой базы, и сравнивает ответ с записью журнала поколений по правилу
`INV.USE-CASES.A-GENERATION-TOKEN-IS-COMPARED-WITHIN-ITS-OWN-TOOL`. Ответ `moved_ahead`
значит, что `push` по этому набору откажет `non_fast_forward`; `unchanged` — что базу с
последнего обмена не меняли.

Источник: [`sources.html`](../../../docs/site/sources.html), раздел о состояниях пары.
