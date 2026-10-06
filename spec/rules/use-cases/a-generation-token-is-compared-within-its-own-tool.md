---
id: INV.USE-CASES.A-GENERATION-TOKEN-IS-COMPARED-WITHIN-ITS-OWN-TOOL
check:
  - src/use_cases/agent_session.rs::the_same_tool_with_the_same_token_is_unchanged
  - src/use_cases/agent_session.rs::the_same_tool_with_another_token_is_changed
  - src/use_cases/agent_session.rs::a_token_of_another_tool_is_no_answer
  - src/use_cases/agent_session.rs::a_record_without_a_tool_is_no_answer_and_spares_the_other_sets
  - tests/cli_dump_agent.rs::a_generation_recorded_by_another_tool_does_not_skip_a_dump
---

# Токен поколения сравнивается только внутри своего инструмента

Запись о поколении в журнале `workPath/infobases/<база>/generation.json` несёт имя
инструмента, которым получен токен: `designer`, `ibcmd` или `agent`. Ответом на вопрос
«менялась ли база» считается только токен того же инструмента. Токен другого инструмента —
отсутствие ответа: он не даёт пропустить работу, не даёт расхождения и не засчитывается как
совпадение. Токен одного и того же расширения у Конфигуратора и `ibcmd` различается
([замер](../../../references/1c/confirmed-runtime-measurements.md)).

Токен пустой базы сравнивается как любой другой: пустую базу по токену раннер не различает.

Запись без имени инструмента — журнал прежних версий раннера — тоже отсутствие ответа; она не
мешает читать записи других наборов и заменяется при следующей записи набора.
