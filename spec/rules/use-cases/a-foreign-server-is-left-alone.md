---
id: INV.USE-CASES.A-FOREIGN-SERVER-IS-LEFT-ALONE
check:
  - src/use_cases/managed_stand/standalone.rs::a_foreign_process_on_a_pinned_port_is_named_and_left_alone
---

# Чужой процесс на портах сервера не трогают

Занятый чужим процессом закреплённый порт — отказ `standalone start` с названием процесса. `standalone stop` такой
процесс называет и оставляет работать.
