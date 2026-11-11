---
id: INV.USE-CASES.A-LONG-AGENT-COMMAND-SHOWS-ITS-PROGRESS
check:
  - tests/cli_progress_events.rs::events_stream_the_agent_progress_and_the_envelope_comes_last_on_one_line
  - tests/cli_progress_events.rs::text_output_shows_the_agent_progress_lines
  - tests/cli_progress_events.rs::without_events_stdout_is_one_envelope
  - tests/cli_progress_events.rs::events_reach_stdout_while_the_agent_still_works
  - src/platform/agent_keeper/client.rs::a_progress_frame_reaches_the_command_before_the_reply_ends
---

# Долгая команда агента показывает ход работы

Ответа агента раннер ждёт столько, сколько агент работает: срок простоя живого агента
ожидание не прерывает (`INV.PLATFORM.A-LIVE-AGENT-CLOSES-WHEN-NOBODY-HOLDS-IT`). Пока ответа
нет, раннер передаёт ход работы: шаги команды и промежуточные сообщения агента — `progress`,
`log` и `loading-issue`, — каждое по мере прихода, а не после итога. Живой агент отдаёт их
через сторожа так же: кадр сторожа доходит до команды, пока агент ещё работает.

Человеку при выводе текстом ход идёт в stdout строками хода агента под узлом этапа
(`CTR.CLI.TEXT-OUTPUT`). При ответе JSON с `--events -` ход идёт в stdout событиями перед
итоговым конвертом (`CTR.WIRE.PROGRESS-EVENTS`); без ключа stdout несёт один конверт, как
прежде. В итоговый JSON ход не попадает ни в том, ни в другом случае.

Текст сообщения агента проходит маску секретов прозы и известных учётных данных: базы, они
же учётные данные SSH агента, СУБД и администраторов кластера.
