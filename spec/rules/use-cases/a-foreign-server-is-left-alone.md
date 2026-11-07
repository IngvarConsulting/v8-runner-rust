---
id: INV.USE-CASES.A-FOREIGN-SERVER-IS-LEFT-ALONE
check:
  - src/use_cases/managed_stand/standalone.rs::a_foreign_process_on_a_pinned_port_is_named_and_left_alone
  - src/use_cases/managed_stand/standalone.rs::a_foreign_server_over_the_same_base_is_named_and_left_alone
  - src/platform/service.rs::a_server_is_found_by_its_own_db_path_only
---

# Чужой сервер не трогают

Чужой `ibsrv` над той же базой или чужой процесс на закреплённом порту — отказ `standalone start` с названием процесса.
`standalone status` и `standalone stop` такой процесс называют и оставляют работать.
