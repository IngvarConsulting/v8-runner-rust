---
id: INV.PLATFORM.AN-AGENT-REPLY-IS-READ-ONLY-AS-JSON
check:
  - src/platform/agent.rs::reply_outcome_is_decided_by_type_and_error_type_not_by_prose
  - src/platform/agent.rs::prose_in_place_of_a_message_array_is_an_invalid_reply
  - src/platform/agent.rs::prose_without_an_array_is_not_a_reply
  - src/platform/agent.rs::a_reply_cut_off_or_left_as_prose_at_the_end_of_the_session_is_an_invalid_reply
  - src/platform/agent.rs::a_message_of_an_unknown_or_missing_type_is_an_invalid_reply
  - src/platform/agent.rs::a_reply_without_a_terminal_message_is_a_refusal
  - src/platform/agent.rs::an_error_without_an_error_type_is_still_a_refusal
  - src/platform/agent.rs::unknown_error_type_is_kept_verbatim
  - src/support/error.rs::an_unreadable_agent_reply_is_invalid_output_and_an_agent_refusal_is_a_platform_failure
  - tests/cli_agent_scenarios.rs::a_prose_reply_of_the_agent_fails_the_command_instead_of_succeeding
---

# Ответ агента читается только как JSON

Итог команды агента решают поля его JSON-массива: тип итогового сообщения и `error-type`.
Текст `message` и проза вне массива итога не решают. Проза без скобки, пока сессия открыта,
ответом не считается: успехом она не становится, и команда ждёт массива дальше.

Нечитаемый ответ — неверный вывод инструмента, род и код `invalid_output`, а не успех: проза
со скобкой, за которой не JSON, сообщение неизвестного типа или без типа, проза или
оборванный массив, на которых сессия кончилась. Сделанное агентом в цель при этом не
переносится.

Разобранный отказ агента — сообщение `error` с `error-type` или без него, с незнакомым
значением, которое сохраняется дословно, — и массив без итогового сообщения остаются отказом
платформы, `platform_failure`.
