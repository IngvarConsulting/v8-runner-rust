---
id: INV.USE-CASES.A-DEFERRED-CANCELLATION-OUTLIVES-A-LATER-FAILURE
check:
  - src/use_cases/load_artifact.rs::a_failed_load_after_a_deferred_cancellation_still_names_it
  - src/use_cases/load_artifact.rs::execute_reports_cancelled_status_at_update_db_cfg_safe_point
  - src/use_cases/infobase_export.rs::a_failed_restore_after_a_deferred_cancellation_still_names_it
  - tests/cli_agent_scenarios.rs::a_failed_restore_through_the_agent_still_names_the_deferred_cancellation
  - src/use_cases/interruption.rs::a_step_names_its_deferrals_whatever_it_ends_with
  - src/use_cases/interruption.rs::a_command_failure_hands_out_its_error_only_with_its_deferral
  - src/use_cases/build_project.rs::a_push_load_that_fails_after_a_deferred_cancellation_names_it
  - src/use_cases/build_project.rs::a_partial_load_that_fails_after_a_deferred_cancellation_names_it
  - src/use_cases/build_project.rs::a_push_stopped_before_the_update_names_the_deferred_load
  - src/use_cases/build_project.rs::an_ibcmd_apply_that_fails_after_a_deferred_cancellation_names_it
  - src/use_cases/build_project.rs::execute_ibcmd_build_honors_interruption_before_apply_safe_point
  - src/use_cases/build_project.rs::a_tool_extension_load_that_fails_after_a_deferred_cancellation_names_it
  - src/use_cases/build_project.rs::a_tool_extension_stopped_before_its_update_names_the_deferred_load
  - src/use_cases/build_project.rs::an_ibcmd_tool_extension_import_that_fails_after_a_deferred_cancellation_names_it
  - tests/cli_build_agent.rs::a_load_that_fails_after_a_deferred_cancellation_names_it
  - tests/cli_build_agent.rs::an_update_that_deferred_the_cancellation_is_named_when_the_generation_is_refused
  - src/use_cases/configure_extensions.rs::a_failed_safety_update_after_a_deferred_cancellation_names_it
  - src/use_cases/reset.rs::a_rollback_that_fails_after_a_deferred_cancellation_names_it
  - src/use_cases/extension_inventory.rs::a_change_through_ibcmd_names_the_cancellation_it_deferred
  - tests/cli_agent_scenarios.rs::extensions_safety_through_the_agent_names_the_deferred_cancellation
  - tests/cli_agent_scenarios.rs::an_extension_created_through_the_agent_names_the_deferral_of_its_failure
  - tests/architecture_guardrails.rs::an_agent_deferral_is_read_in_one_place
  - src/platform/ibcmd.rs::a_create_that_deferred_a_cancel_keeps_its_result_when_the_question_is_refused
  - src/use_cases/init_project.rs::a_failed_creation_after_a_deferred_cancellation_names_it
  - src/use_cases/init_project.rs::a_creation_without_its_marker_after_a_deferred_cancellation_names_it
  - src/use_cases/init_project.rs::an_ibcmd_creation_that_failed_after_a_deferred_cancellation_names_it
  - src/use_cases/init_project.rs::a_stop_after_the_creation_leaves_its_deferred_cancellation_in_the_step
  - tests/architecture_guardrails.rs::a_critical_phase_names_its_deferral_through_the_owner
  - src/use_cases/apply.rs::an_apply_that_fails_after_a_deferred_cancellation_names_it
---

# Отложенная отмена переживает следующий отказ

Отмену, которую критическая фаза отложила до своего исхода, ответ `upload`,
`infobase restore`, `push`, `apply`, `extensions` и `infobase create` называет и тогда,
когда команда кончается не удачей: критическая команда потом отказала, сессия агента
оборвалась, следующая команда не удалась или команда остановилась на следующей безопасной
точке. Так поступает каждый исполнитель этих команд. Форма с `execution` называет её записью о прерывании с
`deferred: true` и тем же текстом среди своих предупреждений — `warnings` у
`infobase restore`, `execution.diagnostics` у `upload`. Форма без `execution` называет
её предупреждением в сообщении шага, чья команда отложила отмену; если этот шаг сам не
удался, предупреждение открывает текст его отказа. Удача после отложенной отмены — предмет
[INV.USE-CASES.AN-INTERRUPTION-STATUS-MEANS-A-TERMINAL-OUTCOME](an-interruption-status-means-a-terminal-outcome.md).
