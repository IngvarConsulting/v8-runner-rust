---
id: INV.USE-CASES.A-DATABASE-WRITE-IS-A-CRITICAL-PHASE
check:
  - src/use_cases/infobase_export.rs::a_cancelled_designer_restore_runs_to_its_end_and_names_the_deferral
  - src/use_cases/build_project.rs::execute_ibcmd_build_honors_interruption_before_apply_safe_point
  - src/use_cases/load_artifact.rs::execute_reports_cancelled_status_at_update_db_cfg_safe_point
  - src/platform/process.rs::a_read_is_never_a_critical_phase
  - src/platform/ibcmd.rs::a_cancel_cuts_the_question_after_a_failed_create
  - src/use_cases/apply.rs::an_apply_stopped_at_its_safe_point_writes_nothing
  - src/use_cases/apply.rs::an_apply_that_fails_after_a_deferred_cancellation_names_it
---

# Запись в базу — критическая фаза

Шаг, который меняет базу, объявляет класс прерывания `CriticalNonAbortable`: загрузка и
применение конфигурации — и у `push`, и у `apply`, — изменение состава и свойств
расширений, создание базы, загрузка базы целиком из файла. Снятый посреди записи процесс оставляет базу в состоянии, которое не
назовёт никто.

Команда, которая базу только читает, критической фазой не бывает, даже когда её задаёт шаг
записи: отмена её снимает. Так снимается вопрос `config generation-id`, которым
`ibcmd infobase create` после неудачи узнаёт, есть ли база уже.
