---
id: INV.USE-CASES.A-MANAGED-STANDALONE-BASE-IS-CREATED-AS-A-FILE-BASE
check:
  - src/config/loader.rs::infobase_create_sees_the_managed_standalone_as_its_file_base_with_the_same_memory
  - tests/cli_standalone_managed.rs::a_preview_of_infobase_create_goes_for_the_file_base_of_the_server
---

# База `standalone.managed` создаётся как файловая

`infobase create` при `db-path` создаёт файловую базу теми же исполнителями, что у файловой цели, и записывает
память под именем базы. Отдельный `ibcmd server config init` раннер не вызывает: `ibsrv --data --db-path` работает над такой
базой (замер #495). Под работающим своим сервером создание отказывает.
