---
id: INV.USE-CASES.A-STANDALONE-STOP-IS-SOFT-FIRST
check:
  - src/use_cases/managed_stand/standalone.rs::stop_is_soft_first_and_confirms_the_end_of_the_process
  - src/use_cases/managed_stand/standalone.rs::a_server_that_ignores_the_soft_stop_is_stopped_hard
---

# `standalone stop` сначала мягкий

Сервер получает закрытие своей консоли, раннер ждёт конца процесса и только затем снимает его принудительно.
Запись снимается после подтверждённого конца.
