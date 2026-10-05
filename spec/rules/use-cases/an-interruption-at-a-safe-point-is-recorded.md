---
id: INV.USE-CASES.AN-INTERRUPTION-AT-A-SAFE-POINT-IS-RECORDED
check:
  - src/use_cases/interruption.rs::a_safe_point_cancel_is_a_cancellation_at_the_boundary
  - src/use_cases/run_tests.rs::run_tests_reports_cancelled_execution_before_first_safe_point
  - src/use_cases/run_tests/helpers.rs::a_build_prerequisite_stopped_by_a_cancellation_is_an_interruption
  - src/use_cases/load_artifact.rs::execute_reports_cancelled_status_before_load_probe
  - src/use_cases/load_artifact.rs::execute_reports_cancelled_status_at_update_db_cfg_safe_point
  - src/use_cases/artifacts.rs::run_artifacts_honors_interruption_before_export_safe_point
  - src/use_cases/infobase_export.rs::a_download_stopped_at_provider_selection_names_the_cancellation
  - src/use_cases/infobase_export.rs::cancelled_process_is_not_collapsed_into_generic_failure
  - src/use_cases/infobase_export.rs::provider_selection_observes_the_operators_interrupt
  - src/use_cases/infobase_export.rs::a_restore_cancelled_before_the_provider_stops_at_the_boundary
---

# Прерывание на безопасной точке записано

В формах `test`, `upload`, `make`, `download`, `infobase dump` и `infobase restore`
прерывание, замеченное на безопасной точке, отвечает статусом `cancelled`, записью в
`execution.interruptions[]` с фазой `command_boundary` и рядом с ней ошибкой в
`execution.errors[]` с кодом `cancelled` — тем же, что код отмены в конверте, — и тем же
текстом, что у записи. Безопасная точка — собственная
проверка команды между шагами или отказ до работы исполнителя: процесс не запущен, команда
запроса не отправлена. Работы команды такое прерывание не обрывает, поэтому фазу работы
запись не называет.
