---
id: DEC.2026-10-03.DETACHED-CLIENT-OWNERSHIP-IS-EXPLICIT
status: active
governs: product
realized:
  - src/platform/process.rs::detached_spawn_survives_wrapper_exit_and_group_cleanup
  - src/platform/process.rs::detached_spawn_cleans_descendant_when_startup_probe_fails
  - src/platform/process.rs::detached_client_dies_with_wrapper_before_startup_handoff
  - src/platform/process.rs::unica_owned_client_survives_handoff_and_failed_start_waits_for_host_cleanup
  - src/platform/process.rs::client_owner_rejects_unknown_value_and_unisolated_runner_before_spawn
  - src/platform/process.rs::host_job_terminates_client_before_startup_handoff
  - src/platform/process.rs::released_host_job_preserves_client_after_runner_exit
establishes: [INV.PLATFORM.DETACHED-CLIENT-OWNERSHIP]
---

# Владение отделяемым клиентом передаётся явно

**Решение.** На Unix обычный отделяемый клиент получает собственную группу процессов.
Ранний отказ запуска очищает эту группу до освобождения лидера. Приватный режим
интеграции сохраняет клиента в группе или Job доверенного вызывающего, который
принимает ответственность за очистку до проверки квитанции. Маркер режима
удаляется из окружения дочерних процессов; runner не создаёт второго владельца.

**Причина.** Ранняя передача клиента в отдельную группу оставляла его живым при
отмене вызывающего до выдачи квитанции. Удержание лидера предотвращает сигнал
по повторно использованному числовому идентификатору группы при раннем отказе.

**Граница.** Проверка отдельной группы или участия в Job подтверждает изоляцию,
а не личность вызывающего. Приватный режим требует явной ответственности
вызывающего за завершение группы или Job при отказе.
