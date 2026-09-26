---
id: INV.USE-CASES.AN-INTERRUPTION-STATUS-MEANS-A-TERMINAL-OUTCOME
check:
  - src/use_cases/infobase_export.rs::cancelled_process_is_not_collapsed_into_generic_failure
  - src/use_cases/infobase_export.rs::unrelated_failure_is_not_reclassified_by_an_interrupted_context
  - src/use_cases/artifacts.rs::an_unrelated_failure_while_an_interruption_is_pending_stays_a_failure
  - src/platform/process.rs::an_interrupted_process_is_reaped_before_the_answer
  - src/platform/interactive.rs::command_timeout_kills_process_and_poison_fails_next_call
  - src/platform/edt_session.rs::execute_blocking_running_cancellation_preserves_cancelled_result_after_forced_cleanup
  - src/platform/edt_session.rs::execute_blocking_running_timeout_preserves_timeout_result_after_forced_cleanup
  - src/use_cases/infobase_export.rs::a_cancelled_designer_restore_runs_to_its_end_and_names_the_deferral
  - src/use_cases/load_artifact.rs::execute_reports_cancelled_status_at_update_db_cfg_safe_point
  - src/platform/process.rs::run_with_policy_defers_timeout_for_critical_process
  - src/use_cases/context.rs::no_process_critical_phase_reports_deferred_cancellation
---

# Статус отмены или истечения предела означает состоявшийся исход

Статусы отмены и истечения предела ставятся только при фактическом терминальном исходе: то,
что запустил шаг, уже не выполняется — завершилось само, остановлено или снято, — а одного
сигнала или истёкшего срока для статуса мало. Критическая фаза, успешно завершившаяся после
сигнала или истечения предела, остаётся успехом с предупреждением об отложенном прерывании;
`tools download` такого предупреждения пока не даёт ([#301](https://github.com/IngvarConsulting/v8-runner-rust/issues/301)).
