---
id: INV.USE-CASES.A-PUSH-INTO-A-BASE-THAT-MOVED-AHEAD-IS-REFUSED
check:
  - tests/cli_push_generation.rs::a_push_into_a_base_that_moved_ahead_is_refused_before_the_load
  - tests/cli_push_generation.rs::a_full_push_into_a_base_that_moved_ahead_is_refused
  - tests/cli_push_generation.rs::a_server_base_that_moved_ahead_may_have_been_changed_by_another_copy
  - tests/cli_push_generation.rs::ibcmd_reads_the_generation_from_its_last_stdout_line
  - tests/cli_build_agent.rs::an_agent_push_into_a_base_that_moved_ahead_is_refused_before_the_load
  - tests/cli_push_generation.rs::a_generation_of_another_tool_refuses_a_push_into_a_base_that_moved_ahead
  - tests/cli_push_generation.rs::an_extension_generation_of_another_tool_is_checked_by_that_tool_before_a_push
  - tests/cli_build_agent.rs::an_extension_recorded_by_ibcmd_is_checked_before_the_agent_session_opens
  - tests/cli_build_agent.rs::an_extension_recorded_by_ibcmd_that_matches_is_loaded_by_the_agent_connected_after_the_check
  - tests/cli_build_agent.rs::a_record_tool_without_an_answer_refuses_the_push
---

# Отправка в ушедшую вперёд базу отклоняется

`push` без `--force`, в том числе полный (`--full`), увидев поколение базы, отличное от
записанного в памяти, отказывает родом `non_fast_forward` до загрузки и называет оба
поколения и следующий шаг. У базы в кластере и у автономного сервера отказ говорит ещё, что
базу могла изменить другая рабочая копия.

Поколение перед загрузкой набора спрашивает тот инструмент, которым набор грузится, —
Конфигуратор (`/GetConfigGenerationID`), `ibcmd` (`config generation-id`) или агент, — если
в памяти лежит запись инструмента, чей токен с его токеном сравним: у основной конфигурации
файловой базы — любого из них, иначе — того же
(`INV.USE-CASES.A-GENERATION-TOKEN-IS-COMPARED-WITHIN-ITS-OWN-TOOL`). Если запись сделал
другой из этих инструментов, а токены несравнимы (расширение, кластер, автономный сервер),
поколение спрашивает инструмент записи — процессом платформы или своей сессией агента — до
первого чтения и до сессии загрузчика, и смена исполнителя между отправками сверку не
снимает. Не ответил инструмент записи — его нет, агент не подключился или ответа нет, — это
отказ `non_fast_forward` до загрузки с причиной и выходом: `pull` набора или `push --force`.
У инструмента загрузчика отсутствие
ответа — ни совпадение, ни расхождение: отказа нет. Когда сверка идёт, держит
`INV.USE-CASES.EVERY-SET-IS-CHECKED-BEFORE-THE-FIRST-LOAD`; что пишется после загрузки —
`INV.USE-CASES.A-LOAD-RECORDS-ITS-GENERATION-OR-ERASES-THE-RECORD` и
`INV.USE-CASES.A-FAILED-LOAD-MARKS-THE-GENERATION-RECORD`.
