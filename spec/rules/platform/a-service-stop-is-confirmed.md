---
id: INV.PLATFORM.A-SERVICE-STOP-IS-CONFIRMED
check:
  - src/use_cases/managed_stand/mod.rs::stop_that_failed_keeps_the_record_and_a_repeat_goes_on
---

# Остановка службы подтверждается концом процесса

Успех `taskkill` без `/F` ничего не доказывает: раннер ждёт конца процесса и только затем снимает его принудительно.
