---
id: INV.CLI.STATUS-DEEP-PREDICTS-THE-PUSH-GENERATION-CHECK
check:
  - tests/cli_status.rs::status_deep_predicts_the_push_generation_check
  - src/use_cases/status.rs::the_verdict_compares_within_the_tool_of_the_record
  - tests/cli_status.rs::status_deep_where_push_does_not_load_with_this_executor_answers_null_with_a_reason
---

# `status --deep` сверяет поколение так, как сверит его `push`

`status --deep` спрашивает поколение каждого набора тем исполнителем, которого `push` выбрал
бы для этой базы, и сравнивает ответ с записью журнала поколений той же сверкой, что `push`
перед загрузкой (`INV.USE-CASES.A-GENERATION-TOKEN-IS-COMPARED-WITHIN-ITS-OWN-TOOL`). Ответ
`moved_ahead` значит, что `push`, который грузит этот набор без `--force`, откажет
`non_fast_forward`; `unchanged` — что базу с последнего обмена не меняли. Где `push` этим
исполнителем проект не грузит, сверки нет: `comparison` — `no_answer` или `no_record`, причина
в `reason`.

Источник: [`sources.html`](../../../docs/site/sources.html), раздел о состояниях пары.
