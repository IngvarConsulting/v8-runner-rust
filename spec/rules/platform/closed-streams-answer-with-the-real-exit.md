---
id: INV.PLATFORM.CLOSED-STREAMS-ANSWER-WITH-THE-REAL-EXIT
check:
  - src/platform/interactive.rs::closed_streams_answer_with_the_reaped_exit_code
  - src/platform/interactive.rs::startup_wait_keeps_the_exit_code_when_streams_close_before_the_exit_is_reported
  - src/platform/interactive.rs::command_wait_keeps_the_exit_code_when_streams_close_before_the_exit_is_reported
  - src/platform/interactive.rs::prompt_wait_reports_an_exit_right_after_the_prompt_before_waitpid_does
  - src/platform/interactive.rs::closed_streams_on_windows_answer_with_the_exit_code_seen_after_them
---

# Закрытые потоки интерактивного процесса отвечают его настоящим исходом

Когда потоки интерактивного процесса закрылись раньше, чем его выход стал виден, исполнитель
отвечает выходом процесса (`ProcessExited`) с кодом, который процесс вернул сам: на ожидании
запуска, на ожидании команды и сразу после подсказки, на Unix и на Windows. Готовность после
подсказки и выдуманный `-1` таким ответом не являются; `-1` получает только процесс, который
сам не вышел и снят исполнителем.
