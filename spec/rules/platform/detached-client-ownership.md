---
id: INV.PLATFORM.DETACHED-CLIENT-OWNERSHIP
check:
  - src/platform/process.rs::detached_spawn_survives_wrapper_exit_and_group_cleanup
  - src/platform/process.rs::detached_spawn_cleans_descendant_when_startup_probe_fails
  - src/platform/process.rs::detached_client_dies_with_wrapper_before_startup_handoff
  - src/platform/process.rs::unica_owned_client_survives_handoff_and_failed_start_waits_for_host_cleanup
  - src/platform/process.rs::client_owner_rejects_unknown_value_and_unisolated_runner_before_spawn
  - src/platform/process.rs::host_job_terminates_client_before_startup_handoff
  - src/platform/process.rs::released_host_job_preserves_client_after_runner_exit
---

# Владение отделяемым клиентом передаётся явно

На Unix обычный `Detached` не принадлежит дереву вызвавшей runner обёртки
после успешного запуска: runner создаёт отдельную группу клиента;
при обнаруженном раннем выходе сигнал очистки этой группы предшествует
освобождению её удерживаемого лидера. Утрата права ожидания лидера
не разрешает сигнал по сохранённому числовому идентификатору.

Приватный режим интеграции позволяет доверенному вызывающему сохранить
владение клиентом до проверки квитанции. Вызывающий явно принимает
ответственность за очистку; runner сохраняет клиент в группе или Job
вызывающего и не создаёт второго владельца. На Unix runner должен быть
лидером отдельной группы, на Windows — участником Job. Проверка этих условий
предшествует запуску клиента; она не удостоверяет личность вызывающего.
Маркер режима удаляется из окружения запускаемых процессов.
