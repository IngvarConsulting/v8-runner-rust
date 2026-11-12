---
id: INV.USE-CASES.A-LOAD-RECORDS-ITS-GENERATION-OR-ERASES-THE-RECORD
check:
  - tests/cli_push_generation.rs::a_push_force_loads_without_memory_and_remembers_the_generation
  - tests/cli_push_generation.rs::without_an_answer_after_the_load_the_record_is_erased_and_named
  - src/use_cases/build_project.rs::a_cancellation_while_reading_the_generation_after_a_load_stops_the_step
  - src/use_cases/apply.rs::an_apply_without_an_answer_of_the_record_tool_erases_the_record
  - src/use_cases/build_project.rs::a_load_asks_the_generation_after_itself_only_when_the_answer_decides_something
  - src/use_cases/exchange_guard.rs::the_generation_after_a_load_is_asked_only_when_it_decides_something
---

# После загрузки поколение записывается или запись стирается

После удачной загрузки набора поколение спрашивается снова тем же инструментом и
записывается с его именем. Без ответа запись набора стирается, чтобы своя загрузка не
выглядела чужой правкой, и ответ это называет. Отмена, замеченная при этом чтении,
останавливает шаг у любого исполнителя одинаково: запись стирается, и ответ называет и
отмену, и стёртую запись. Набор без журнала поколений (памяти о базе нет) поколения после
загрузки не спрашивает: записывать некуда, а свежий файл версий
(`INV.USE-CASES.A-PUSH-REFRESHES-THE-VERSION-FILE-ONLY-WHEN-IT-PAYS`) без памяти о базе не
записать.
Так же после применения: `apply` спрашивает поколение инструментом записи и переносит на
ответ совпавшую с ним запись, а без ответа стирает её и это называет.
