---
id: INV.USE-CASES.A-FOREIGN-SERVER-IS-LEFT-ALONE
check:
  - src/use_cases/managed_stand/standalone.rs::a_foreign_process_on_a_pinned_port_is_named_and_left_alone
  - src/use_cases/managed_stand/standalone.rs::a_foreign_server_over_the_same_base_is_named_and_left_alone
  - src/use_cases/managed_stand/standalone.rs::a_foreign_server_over_the_same_database_is_found_by_its_keys
  - src/platform/service.rs::a_server_is_found_by_all_of_its_keys_only
  - src/platform/service.rs::a_server_is_found_by_any_spelling_of_its_keys
  - src/platform/service.rs::a_trailing_backslash_before_the_quote_is_one_backslash
---

# Чужой сервер не трогают

Чужой `ibsrv` над той же базой или чужой процесс на закреплённом порту — отказ `standalone start` с названием процесса.
`standalone status` и `standalone stop` такой процесс называют и оставляют работать.
Сервер над базой в СУБД узнают по адресу сервера СУБД и имени БД так, как они записаны в его команде: другая запись
того же адреса, например `localhost` вместо `127.0.0.1`, — другой адрес.
