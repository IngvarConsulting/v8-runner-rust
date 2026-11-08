---
id: INV.PLATFORM.FIRST-PORTS-COME-FROM-THE-COPY-PATH
check:
  - src/use_cases/managed_stand/mod.rs::a_first_port_taken_by_someone_is_chosen_again
---

# Первый порт выводится из пути рабочей копии

При первом запуске порт выбирается вне динамического диапазона и выводится из пути рабочей копии, чтобы копии не
мешали друг другу. Если выбранный порт занят, выбор при этом запуске повторяется.
