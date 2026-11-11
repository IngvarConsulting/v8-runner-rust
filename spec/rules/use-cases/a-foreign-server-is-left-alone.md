---
id: INV.USE-CASES.A-FOREIGN-SERVER-IS-LEFT-ALONE
check:
  - src/use_cases/managed_stand/standalone.rs::a_taken_pinned_block_moves_the_server_to_a_new_block_with_the_same_key
  - src/use_cases/managed_stand/standalone.rs::a_foreign_server_over_the_same_base_is_named_and_left_alone
  - src/use_cases/managed_stand/standalone.rs::a_foreign_server_over_the_same_database_is_found_by_its_keys
  - src/platform/service.rs::a_server_is_found_by_all_of_its_keys_only
  - src/platform/service.rs::a_server_is_found_by_any_spelling_of_its_keys
  - src/platform/service.rs::a_trailing_backslash_before_the_quote_is_one_backslash
---

# Чужой сервер не трогают

Чужой `ibsrv` над той же базой — отказ `standalone start` с названием процесса. Чужой процесс на закреплённом порту
запуск обходит новым блоком портов (`INV.PLATFORM.STANDALONE-PORTS-AND-KEY-ARE-PINNED`) и не трогает. `standalone status`
и `standalone stop` такие процессы называют и оставляют работать.
Сервер над базой в СУБД узнают по адресу сервера СУБД и имени БД так, как они записаны в его команде: другая запись
того же адреса, например `localhost` вместо `127.0.0.1`, — другой адрес.
