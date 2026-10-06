---
id: INV.USE-CASES.A-PUSH-INTO-A-BASE-THAT-MOVED-AHEAD-IS-REFUSED
check:
  - tests/cli_push_generation.rs::a_push_into_a_base_that_moved_ahead_is_refused_before_the_load
  - tests/cli_push_generation.rs::a_full_push_into_a_base_that_moved_ahead_is_refused
  - tests/cli_push_generation.rs::a_server_base_that_moved_ahead_may_have_been_changed_by_another_copy
  - tests/cli_push_generation.rs::ibcmd_reads_the_generation_from_its_last_stdout_line
  - tests/cli_push_generation.rs::every_set_is_checked_before_the_first_load
  - tests/cli_push_generation.rs::after_a_failed_load_the_next_push_names_it_and_is_not_let_through
  - tests/cli_push_generation.rs::without_an_answer_after_the_load_the_record_is_erased_and_named
  - tests/cli_build_agent.rs::an_agent_push_into_a_base_that_moved_ahead_is_refused_before_the_load
---

# Отправка в ушедшую вперёд базу отклоняется

`push` без `--force`, в том числе полный (`--full`), увидев поколение базы, отличное от
записанного в памяти, отказывает родом `non_fast_forward` до загрузки и называет оба
поколения и следующий шаг. У базы в кластере и у автономного сервера отказ говорит ещё, что
базу могла изменить другая рабочая копия.

Поколение перед загрузкой набора спрашивает тот инструмент, которым набор грузится, —
Конфигуратор (`/GetConfigGenerationID`), `ibcmd` (`config generation-id`) или агент, — и
только если в памяти лежит запись того же инструмента: сравнивать токены разных инструментов
нельзя (`INV.USE-CASES.A-GENERATION-TOKEN-IS-COMPARED-WITHIN-ITS-OWN-TOOL`). Отсутствие
ответа — ни совпадение, ни расхождение: отказа нет. Когда в базу идут несколько наборов,
поколение каждого сверяется до первой загрузки команды: отказ по набору не приходит после
того, как наборы перед ним уже загружены.

После удачной загрузки поколение спрашивается снова и записывается с именем инструмента; без
ответа запись набора стирается, чтобы своя загрузка не выглядела чужой правкой, и ответ это
называет. После неудачной загрузки поколение не записывается, и ответ это называет; запись
того же инструмента помечается как сделанная перед неудачной загрузкой. Если поколение с ней
разойдётся, следующая отправка отказывает `non_fast_forward` и говорит, что базу изменила
неудачная загрузка или другая копия, а не что база ушла вперёд с прошлого обмена.
