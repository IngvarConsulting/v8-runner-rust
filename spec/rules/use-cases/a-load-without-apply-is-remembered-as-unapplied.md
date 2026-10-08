---
id: INV.USE-CASES.A-LOAD-WITHOUT-APPLY-IS-REMEMBERED-AS-UNAPPLIED
check:
  - tests/cli_apply.rs::a_push_without_apply_loads_and_apply_applies_it
  - tests/cli_apply.rs::a_push_after_an_apply_that_changed_the_generation_is_not_refused
  - tests/cli_apply.rs::a_pull_after_a_push_without_apply_is_up_to_date
  - src/use_cases/agent_session.rs::a_record_without_the_applied_mark_reads_as_applied
  - src/use_cases/apply.rs::an_apply_reads_the_generation_with_the_tool_of_the_record
  - src/use_cases/apply.rs::an_apply_without_an_answer_of_the_record_tool_erases_the_record
---

# Загрузка без применения запоминается как непринятая

Запись поколения после загрузки, которую не применили, несёт отдельный признак `applied:
false`; поле `after` по-прежнему называет операцию. Раннер, который признака не знает,
читает запись как обычную запись отправки и проверку `non_fast_forward` не теряет. Признаку
верят, только пока поколение базы равно записи. Удачное применение тем же инструментом
снимает признак и записывает новое поколение; без ответа после применения запись стирается
и ответ это называет — как после загрузки
(`INV.USE-CASES.A-LOAD-RECORDS-ITS-GENERATION-OR-ERASES-THE-RECORD`).
