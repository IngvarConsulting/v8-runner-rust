---
id: INV.USE-CASES.MAKE-STOPS-AT-THE-SAFE-POINTS-OF-ITS-BUILD
check:
  - src/use_cases/artifacts.rs::run_artifacts_honors_interruption_before_export_safe_point
  - src/use_cases/artifacts.rs::a_cancellation_during_creation_stops_before_the_load_and_the_base_is_removed
  - src/use_cases/artifacts.rs::a_cancellation_during_the_load_stops_before_the_dump
  - src/use_cases/artifacts.rs::designer_export_interruption_before_publish_retains_stage_artifact
  - src/use_cases/artifacts.rs::a_designer_export_cancelled_after_its_start_is_a_cut_provider_command
---

# Отмена останавливает `make` на безопасной точке сборки

Отмена, пришедшая до сборки, останавливает `make` до создания временной базы; пришедшая во
время создания — перед загрузкой исходников; во время загрузки — перед выгрузкой пакета;
во время выгрузки — перед публикацией, и промежуточная копия остаётся. Запись во временную
базу критической фазой не является: процесс исполнителя, снятый отменой посреди работы, —
оборванная работа команды (`provider_command`), а не отложенная отмена.
