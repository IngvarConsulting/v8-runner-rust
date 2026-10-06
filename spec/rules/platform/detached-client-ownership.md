---
id: INV.PLATFORM.DETACHED-CLIENT-OWNERSHIP
check:
  - src/platform/process.rs::detached_spawn_survives_wrapper_exit_and_group_cleanup
  - src/platform/process.rs::detached_spawn_cleans_descendant_when_startup_probe_fails
  - src/platform/process.rs::startup_observation_error_preserves_descendant_after_leader_was_reaped
  - src/platform/process.rs::an_interruption_after_the_leader_was_reaped_elsewhere_signals_nothing_more
---


# Отделяемый Unix-клиент получает собственную группу

На Unix обычный `Detached` не принадлежит дереву вызвавшей runner обёртки
после успешного запуска: runner создаёт отдельную группу клиента;
при обнаруженном раннем выходе сигнал очистки этой группы предшествует
освобождению её удерживаемого лидера. Утрата права ожидания лидера
не разрешает сигнал по сохранённому числовому идентификатору.
