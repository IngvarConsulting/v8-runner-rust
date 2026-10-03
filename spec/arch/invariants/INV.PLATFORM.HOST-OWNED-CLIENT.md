---
id: INV.PLATFORM.HOST-OWNED-CLIENT
status: active
governs: product
decision: DEC.2026-10-03.DETACHED-CLIENT-OWNERSHIP-IS-EXPLICIT
check:
  - src/platform/process.rs::detached_client_dies_with_wrapper_before_startup_handoff
  - src/platform/process.rs::unica_owned_client_survives_handoff_and_failed_start_waits_for_host_cleanup
  - src/platform/process.rs::client_owner_rejects_unknown_value_and_unisolated_runner_before_spawn
  - src/platform/process.rs::host_job_terminates_client_before_startup_handoff
  - src/platform/process.rs::released_host_job_preserves_client_after_runner_exit
scope: [platform]
---

# Клиент остаётся во владении вызывающего до квитанции

Приватный режим интеграции позволяет доверенному вызывающему сохранить
владение клиентом до проверки квитанции. Вызывающий явно принимает
ответственность за очистку; runner сохраняет клиент в группе или Job
вызывающего и не создаёт второго владельца. На Unix runner должен быть
лидером отдельной группы, на Windows — участником Job. Проверка этих условий
предшествует запуску клиента; она не удостоверяет личность вызывающего.
Маркер режима удаляется из окружения запускаемых процессов.
