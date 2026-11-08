---
id: INV.USE-CASES.A-GENERATION-TOKEN-IS-COMPARED-WITHIN-ITS-OWN-TOOL
check:
  - src/use_cases/agent_session.rs::the_same_tool_with_the_same_token_is_unchanged
  - src/use_cases/agent_session.rs::the_same_tool_with_another_token_is_changed
  - src/use_cases/agent_session.rs::a_token_of_another_tool_is_no_answer
  - src/use_cases/agent_session.rs::a_token_of_another_measured_tool_is_compared_where_tokens_are_shared
  - src/use_cases/agent_session.rs::a_record_without_a_tool_is_no_answer_and_spares_the_other_sets
  - src/use_cases/agent_session.rs::an_empty_base_token_is_compared_like_any_other
  - src/use_cases/status.rs::the_verdict_compares_within_the_tool_of_the_record
  - tests/cli_dump_agent.rs::a_generation_of_another_tool_skips_a_dump_and_a_record_without_a_tool_does_not
  - tests/cli_push_generation.rs::a_generation_of_another_tool_refuses_a_push_into_a_base_that_moved_ahead
  - tests/cli_push_generation.rs::an_extension_generation_of_another_tool_is_checked_by_that_tool_before_a_push
  - tests/cli_push_generation.rs::an_unapplied_mark_survives_a_dump_by_another_tool
  - src/platform/generation.rs::an_unrecognized_answer_is_no_answer
---

# Токен поколения сравнивается внутри своего инструмента, а у основной конфигурации файловой базы — между агентом, Конфигуратором и `ibcmd`

У основной конфигурации файловой базы токен, записанный любым из агента, Конфигуратора и
`ibcmd`, сравнивается с токеном, прочитанным любым другим из них: совпадение — совпадение,
расхождение — расхождение. Так сравнивают сверка перед загрузкой, пропуск выгрузки по
неизменившемуся поколению, признак непринятого после выгрузки и ответ `status --deep`.

У расширения, у базы в кластере и у автономного сервера токен сравнивается только с токеном
того же инструмента; токен другого инструмента или запись без имени инструмента —
отсутствие ответа: он не даёт пропустить работу, не даёт расхождения и не засчитывается как
совпадение. Поэтому сверка перед загрузкой спрашивает поколение у инструмента записи, когда
грузит другой, а его молчание — отказ: так смена исполнителя её не снимает
(`INV.USE-CASES.A-PUSH-INTO-A-BASE-THAT-MOVED-AHEAD-IS-REFUSED`). Запись без имени
инструмента — отсутствие ответа и у основной конфигурации файловой базы. Токен пустой базы
сравнивается как любой другой.

Ответ Конфигуратора и `ibcmd` — последняя непустая строка, если это токен из шестнадцатеричных знаков
(`/Out` у Конфигуратора, stdout у `ibcmd`); что иначе — отсутствие ответа. Как разбирается
ответ Конфигуратора у базы в кластере, держит
`INV.USE-CASES.A-CLUSTER-DESIGNER-GENERATION-IS-READ-LIKE-A-FILE-ONE`.

## Замер

Общий токен основной конфигурации замерен 08.11.2026 на файловых базах платформ 8.3.24,
8.3.27 и 8.5.4 в состояниях «как есть», «после применённой правки», «загружено, не
применено» и «применено снова»: токены агента, Конфигуратора и `ibcmd` совпали везде
([замер](../../../references/1c/confirmed-runtime-measurements.md), раздел «Общий токен
поколения у агента, Конфигуратора и ibcmd (#559)»). Токен одного и того же расширения у
Конфигуратора и `ibcmd` различается: один читает загруженное, другой применённое. Кластер и
автономный сервер не замерены.
