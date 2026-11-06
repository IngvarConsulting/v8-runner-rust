---
id: INV.USE-CASES.MAKE-STOPS-AT-THE-SAFE-POINTS-OF-ITS-BUILD
check:
  - src/use_cases/artifacts.rs::run_artifacts_honors_interruption_before_export_safe_point
  - src/use_cases/artifacts.rs::a_cancellation_during_creation_stops_before_load_and_leaves_no_ready_memory
  - src/use_cases/artifacts.rs::a_cancellation_during_the_load_stops_before_the_dump
  - src/use_cases/artifacts.rs::designer_export_interruption_before_publish_retains_stage_artifact
  - src/use_cases/artifacts.rs::a_designer_export_cancelled_after_its_start_is_a_cut_provider_command
---

# Отмена останавливает `make` на безопасной точке сборки

Отмена до сборки останавливает `make` перед созданием базы. Начатое создание
или загрузка в постоянную базу завершается в критической фазе. Раннер сообщает
об отложенной отмене и останавливается перед следующим шагом. Если готовность
базы уже снята, следующий прогон создаёт свежую базу.

Выгрузка пакета прерываема. Остановка процесса исполнителя во время выгрузки
называется `provider_command`. Отмена после выгрузки останавливает публикацию
и оставляет промежуточную копию.
