---
id: INV.USE-CASES.A-DATABASE-WRITE-IS-A-CRITICAL-PHASE
check:
  - src/use_cases/infobase_export.rs::a_cancelled_designer_restore_runs_to_its_end_and_names_the_deferral
  - src/use_cases/build_project.rs::execute_ibcmd_build_honors_interruption_before_apply_safe_point
  - src/use_cases/load_artifact.rs::execute_reports_cancelled_status_at_update_db_cfg_safe_point
---

# Запись в базу — критическая фаза

Шаг, который меняет базу, объявляет класс прерывания `CriticalNonAbortable`: загрузка и
применение конфигурации, изменение состава и свойств расширений, создание базы, загрузка
базы целиком из файла. Снятый посреди записи процесс оставляет базу в состоянии, которое не
назовёт никто.
