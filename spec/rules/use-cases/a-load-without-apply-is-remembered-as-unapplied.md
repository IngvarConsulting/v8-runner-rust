---
id: INV.USE-CASES.A-LOAD-WITHOUT-APPLY-IS-REMEMBERED-AS-UNAPPLIED
check:
  - tests/cli_apply.rs::a_push_without_apply_loads_and_apply_applies_it
  - tests/cli_apply.rs::a_push_after_an_apply_that_changed_the_generation_is_not_refused
  - tests/cli_apply.rs::a_pull_after_a_push_without_apply_is_up_to_date
  - tests/cli_apply.rs::a_push_without_apply_and_without_a_generation_leaves_no_hash_memory
  - src/use_cases/build_project.rs::execute_ibcmd_build_honors_interruption_before_apply_safe_point
  - tests/cli_apply.rs::an_extension_whose_generation_moves_only_on_apply_is_pushed_again_without_refusal
  - src/use_cases/agent_session.rs::a_record_without_the_applied_mark_reads_as_applied
  - src/use_cases/apply.rs::an_apply_reads_the_generation_with_the_tool_of_the_record
  - src/use_cases/apply.rs::an_apply_without_an_answer_of_the_record_tool_erases_the_record
  - tests/cli_reset.rs::reset_rewrites_the_record_with_the_tool_that_made_it
  - tests/cli_reset.rs::reset_without_an_answer_erases_the_record
  - tests/cli_reset.rs::reset_into_a_base_that_moved_keeps_the_record_and_the_next_push_is_refused
  - tests/cli_reset.rs::reset_keeps_a_record_made_before_a_failed_load
---

# Загрузка без применения запоминается как непринятая

Запись поколения после загрузки, которую не применили, несёт отдельный признак `applied:
false`; поле `after` по-прежнему называет операцию. Память исходников после такой загрузки
фиксируется, только когда запись действительно легла в журнал: без ответа инструмента, при
сбое записи или без журнала она не фиксируется, и следующая отправка загрузит и применит
набор снова. Раннер, который признака не знает,
читает запись как обычную запись отправки и проверку `non_fast_forward` не теряет. Признаку
верят, только пока поколение базы равно записи. Удачное применение тем же инструментом
снимает признак и записывает новое поколение; без ответа после применения запись стирается
и ответ это называет — как после загрузки
(`INV.USE-CASES.A-LOAD-RECORDS-ITS-GENERATION-OR-ERASES-THE-RECORD`). Откат непринятого
(`reset`) снимает признак так же: поколение до и после отката читает инструмент записи; база
не ушла от записи — запись переписывается его ответом с прежним `after`; ушла, или запись
сделана перед неудачной загрузкой, — запись не трогается; без ответа она стирается.
