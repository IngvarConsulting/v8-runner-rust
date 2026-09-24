---
id: INV.USE-CASES.CANCELLED-MEANS-TERMINAL-CANCELLATION
check:
  - src/use_cases/infobase_export.rs::cancelled_process_is_not_collapsed_into_generic_failure
  - src/use_cases/infobase_export.rs::unrelated_failure_is_not_reclassified_by_an_interrupted_context
  - src/platform/edt_session.rs::execute_blocking_running_cancellation_preserves_cancelled_result_after_forced_cleanup
  - src/use_cases/infobase_export.rs::a_cancelled_designer_restore_runs_to_its_end_and_names_the_deferral
  - src/use_cases/context.rs::no_process_critical_phase_reports_deferred_cancellation
---

# Статус отмены означает состоявшуюся отмену

Статус отмены ставится только при фактической терминальной отмене: то, что запустил шаг,
уже не выполняется — завершилось само, остановлено или снято, — а одного сигнала для
статуса мало. Критическая фаза, успешно завершившаяся после сигнала, остаётся успехом с
предупреждением об отложенном прерывании.
