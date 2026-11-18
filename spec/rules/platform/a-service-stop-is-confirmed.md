---
id: INV.PLATFORM.A-SERVICE-STOP-IS-CONFIRMED
check:
  - src/use_cases/managed_stand/mod.rs::stop_that_failed_keeps_the_record_and_a_repeat_goes_on
  - src/use_cases/managed_stand/debug_server.rs::stop_stops_the_server_of_its_session_and_repeats_safely
---

# Остановка службы подтверждается концом процесса

Успех `taskkill` без `/F` ничего не доказывает: раннер ждёт конца процесса и только затем снимает его принудительно.
