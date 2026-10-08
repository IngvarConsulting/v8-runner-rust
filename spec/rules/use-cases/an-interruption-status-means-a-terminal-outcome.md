---
id: INV.USE-CASES.AN-INTERRUPTION-STATUS-MEANS-A-TERMINAL-OUTCOME
check:
  - src/use_cases/infobase_export.rs::cancelled_process_is_not_collapsed_into_generic_failure
  - src/use_cases/infobase_export.rs::unrelated_failure_is_not_reclassified_by_an_interrupted_context
  - src/use_cases/artifacts.rs::an_unrelated_failure_while_an_interruption_is_pending_stays_a_failure
  - src/platform/process.rs::an_interrupted_process_is_reaped_before_the_answer
  - src/platform/process.rs::an_interruption_answers_only_after_a_confirmed_end
  - src/platform/process.rs::an_interruption_is_a_cancellation_when_sigchld_is_ignored_by_inheritance
  - src/platform/interactive.rs::command_timeout_kills_process_and_poison_fails_next_call
  - src/platform/edt_session.rs::execute_blocking_running_cancellation_preserves_cancelled_result_after_forced_cleanup
  - src/platform/edt_session.rs::execute_blocking_running_timeout_preserves_timeout_result_after_forced_cleanup
  - src/platform/edt.rs::shared_session_defers_the_interruption_of_a_critical_step
  - src/use_cases/infobase_export.rs::a_cancelled_designer_restore_runs_to_its_end_and_names_the_deferral
  - src/use_cases/load_artifact.rs::execute_reports_cancelled_status_at_update_db_cfg_safe_point
  - src/platform/process.rs::run_with_policy_defers_timeout_for_critical_process
  - src/use_cases/context.rs::no_process_critical_phase_reports_deferred_cancellation
  - src/use_cases/configure_extensions.rs::a_safety_update_that_deferred_the_cancellation_names_it
  - src/use_cases/extension_inventory.rs::a_change_through_ibcmd_names_the_cancellation_it_deferred
  - tests/cli_agent_scenarios.rs::extensions_safety_through_the_agent_names_the_deferred_cancellation
  - src/use_cases/init_project.rs::a_created_infobase_names_the_cancellation_its_creation_deferred
  - src/use_cases/apply.rs::a_cancellation_deferred_by_the_last_apply_is_named_once
  - src/use_cases/build_project.rs::a_cancellation_deferred_by_applying_the_unapplied_is_named_once
---

# Статус отмены или истечения предела означает состоявшийся исход

Статусы отмены и истечения предела ставятся только при фактическом терминальном исходе: то,
что запустил шаг, уже не выполняется — завершилось само, остановлено или снято, — а одного
сигнала или истёкшего срока для статуса мало. Критическая фаза, успешно завершившаяся после
сигнала или истечения предела, остаётся успехом с предупреждением об отложенном прерывании;
`tools download` такого предупреждения пока не даёт ([#301](https://github.com/IngvarConsulting/v8-runner-rust/issues/301)).
Отмена команды у агента, к которому раннер подключился, и у шлюза SSH — предмет
[INV.USE-CASES.AN-ATTACHED-AGENT-ANSWERS-CANCELLED-AFTER-ITS-COMMAND-ENDS](an-attached-agent-answers-cancelled-after-its-command-ends.md).
