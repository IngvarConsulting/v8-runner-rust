---
id: INV.USE-CASES.A-GENERATION-TOKEN-IS-COMPARED-WITHIN-ITS-OWN-TOOL
check:
  - src/use_cases/agent_session.rs::the_same_tool_with_the_same_token_is_unchanged
  - src/use_cases/agent_session.rs::the_same_tool_with_another_token_is_changed
  - src/use_cases/agent_session.rs::a_token_of_another_tool_is_no_answer
  - src/use_cases/agent_session.rs::a_record_without_a_tool_is_no_answer_and_spares_the_other_sets
  - src/use_cases/agent_session.rs::an_empty_base_token_is_compared_like_any_other
  - tests/cli_dump_agent.rs::a_generation_recorded_by_another_tool_does_not_skip_a_dump
---

# Токен поколения сравнивается только внутри своего инструмента

Токен поколения сравнивается только с токеном того же инструмента; токен другого
инструмента или запись без имени инструмента — отсутствие ответа: он не даёт пропустить
работу, не даёт расхождения и не засчитывается как совпадение. Токен одного и того же
расширения у Конфигуратора и `ibcmd` различается
([замер](../../../references/1c/confirmed-runtime-measurements.md)). Токен пустой базы
сравнивается как любой другой.

Конфигуратор и `ibcmd` поколение пока не читают и не пишут —
[#215](https://github.com/IngvarConsulting/v8-runner-rust/issues/215).
