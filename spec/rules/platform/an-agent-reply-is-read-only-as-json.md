---
id: INV.PLATFORM.AN-AGENT-REPLY-IS-READ-ONLY-AS-JSON
check:
  - src/platform/agent.rs::reply_outcome_is_decided_by_type_and_error_type_not_by_prose
  - src/platform/agent.rs::prose_in_place_of_a_message_array_is_an_invalid_reply
  - src/platform/agent.rs::prose_without_an_array_is_not_a_reply
  - src/platform/agent.rs::a_message_of_an_unknown_or_missing_type_is_an_invalid_reply
  - src/platform/agent.rs::a_reply_without_a_terminal_message_is_a_refusal
  - src/platform/agent.rs::an_error_without_an_error_type_is_still_a_refusal
  - src/platform/agent.rs::unknown_error_type_is_kept_verbatim
  - tests/cli_agent_scenarios.rs::a_prose_reply_of_the_agent_fails_the_command_instead_of_succeeding
---

# Ответ агента читается только как JSON

Итог команды агента решают поля его JSON-массива: тип итогового сообщения и `error-type`.
Текст `message` и проза вне массива итога не решают: проза до массива ответом не считается.

Что не разбирается как массив сообщений известного типа — проза со скобкой, сообщение
неизвестного типа или без типа, — даёт отказ, а не успех. Массив без итогового сообщения —
тоже отказ. Ошибка без `error-type` или с `error-type` вне закрытого множества остаётся
отказом, а незнакомое значение сохраняется дословно.

Команда, получившая от агента прозу вместо итога, отвечает `platform_failure`, и сделанное
агентом в цель не переносится.
