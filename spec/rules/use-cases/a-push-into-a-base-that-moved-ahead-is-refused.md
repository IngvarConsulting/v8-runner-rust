---
id: INV.USE-CASES.A-PUSH-INTO-A-BASE-THAT-MOVED-AHEAD-IS-REFUSED
check:
  - tests/cli_push_generation.rs::a_push_into_a_base_that_moved_ahead_is_refused_before_the_load
  - tests/cli_push_generation.rs::a_server_base_that_moved_ahead_may_have_been_changed_by_another_copy
  - tests/cli_push_generation.rs::ibcmd_reads_the_generation_from_its_last_stdout_line
  - tests/cli_build_agent.rs::an_agent_push_into_a_base_that_moved_ahead_is_refused_before_the_load
---

# Отправка в ушедшую вперёд базу отклоняется

`push` без `--force`, увидев поколение базы, отличное от записанного в памяти,
отказывает родом `non_fast_forward` до загрузки и называет оба поколения и следующий
шаг. У базы в кластере и у автономного сервера отказ говорит ещё, что базу могла изменить
другая рабочая копия.

Поколение перед загрузкой набора спрашивает тот инструмент, которым набор грузится, —
Конфигуратор (`/GetConfigGenerationID`), `ibcmd` (`config generation-id`) или агент, — и
только если в памяти лежит запись того же инструмента: сравнивать токены разных инструментов
нельзя (`INV.USE-CASES.A-GENERATION-TOKEN-IS-COMPARED-WITHIN-ITS-OWN-TOOL`). Отсутствие
ответа — ни совпадение, ни расхождение: отказа нет. После удачной загрузки поколение
спрашивается снова и записывается с именем инструмента; без ответа запись набора стирается,
чтобы своя загрузка не выглядела чужой правкой.
