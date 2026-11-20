---
id: INV.PLATFORM.A-SERVICE-STOP-IS-CONFIRMED
check:
  - src/platform/service.rs::a_stalled_soft_stop_helper_is_terminated_at_its_deadline
  - src/use_cases/managed_stand/mod.rs::stop_that_failed_keeps_the_record_and_a_repeat_goes_on
  - src/use_cases/managed_stand/debug_server.rs::stop_stops_the_server_of_its_session_and_repeats_safely
---

# Остановка службы подтверждается концом процесса

Мягкий `taskkill` имеет собственный предел ожидания. При его истечении раннер снимает саму утилиту.
Конец службы раннер подтверждает отдельно. Если за отведённое время служба не вышла, раннер останавливает её принудительно и проверяет снова.
