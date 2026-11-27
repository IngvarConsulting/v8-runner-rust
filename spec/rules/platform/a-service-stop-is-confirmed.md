---
id: INV.PLATFORM.A-SERVICE-STOP-IS-CONFIRMED
check:
  - src/platform/service.rs::a_stalled_soft_stop_helper_is_terminated_at_its_deadline
  - src/use_cases/managed_stand/mod.rs::stop_that_failed_keeps_the_record_and_a_repeat_goes_on
  - src/use_cases/managed_stand/debug_server.rs::prepared_stop_waits_for_death_after_identity_becomes_unavailable
  - tests/cli_debug_server.rs::start_runs_the_debug_server_beside_the_client_and_stop_ends_it
---

# Остановка службы подтверждается концом процесса

Мягкий `taskkill` имеет собственный предел ожидания. При его истечении раннер снимает саму утилиту.
Конец службы раннер подтверждает отдельно. Если за отведённое время служба не вышла, раннер останавливает её принудительно и проверяет снова.
