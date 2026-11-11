---
id: INV.USE-CASES.AN-ATTACHED-AGENT-ANSWERS-CANCELLED-AFTER-ITS-COMMAND-ENDS
check:
  - tests/cli_agent_standalone.rs::cancellation_of_an_external_agent_waits_for_a_terminal_reply
  - tests/cli_agent_standalone.rs::a_second_interrupt_abandons_the_wait_for_an_external_agent
  - src/mcp/server.rs::a_repeated_cancellation_of_a_call_reaches_it_as_the_second_signal
  - tests/cli_agent_standalone.rs::a_ctrl_break_from_another_process_group_cancels_like_ctrl_c
---

# Чужой агент отвечает отменой после конца своей команды

Команда, которую исполняет агент Конфигуратора, к которому раннер подключился (`attached`),
или шлюз SSH автономного сервера (`gate`), получает ответ `cancelled` только после
наблюдаемого конца этой команды на стороне агента. Разрыв сессии команду не снимает, поэтому
после отмены уже отправленной некритической команды раннер сессию не рвёт: он пишет в журнал,
что ждёт итог агента и как перестать ждать, и читает ответ до итога. Итог `success` или явный
`canceled` агента дают `cancelled`; отказ агента, вопрос и потеря канала до итога сохраняют
свои ошибки. Результат прерванной выгрузки не публикуется.

Ожидание ограничивает второй сигнал: второе прерывание командной строки (`Ctrl+C`, `SIGTERM`
на Unix, `CTRL_BREAK` на Windows) не раньше секунды после первого, у MCP —
повторное `notifications/cancelled` для того же запроса. Тогда раннер перестаёт ждать и
отвечает конвертом с кодом `agent_command_abandoned` рода `platform` (выход 4, у MCP —
`platform_failure`): конец команды не наблюдался, агент, возможно, ещё её выполняет и держит
базу. Ответ `cancelled` в этом случае не даётся, а след прогона на стороне агента раннер не
убирает. Критическую фазу второй сигнал не прерывает: её исход ждут всегда. Свой агент и
служебные команды сессии сохраняют свои политики. Режим HTTP без сессий
(`mcp.http.stateful_sessions: false`) отмену запроса не доставляет вовсе, и второго сигнала
там нет. Отмену, которую сервер получил раньше, чем вызов начал считать отмены, второй
сигнал не засчитывает.

Замеры путей `attached` и `gate` — в
[замерах платформы](../../../references/1c/confirmed-runtime-measurements.md#отмена-у-чужого-агента-и-шлюза-ssh).
