---
id: INV.USE-CASES.A-MANAGED-RECREATE-DROPS-ONLY-ITS-OWN-DATABASE
check:
  - src/use_cases/managed_stand/cluster.rs::a_managed_recreate_sets_the_database_aside_and_drops_only_it
  - src/use_cases/managed_stand/cluster.rs::a_foreign_dbms_and_a_leftover_database_are_refused
---

# Пересоздание стенда удаляет только свою БД

`infobase create --recreate` серверной базы работает только у стенда раннера над его
PostgreSQL: `cluster.managed` или `standalone.managed` вместе с `dbms.managed`. Кластер без
`managed`, автономный сервер без `managed` — отказ рода `capability` с кодом `target`; стенд
над вашей СУБД — отказ `validation`. Такую базу удаляют своими средствами.

У стенда базу откладывают так: регистрацию в кластере раннера снимает `rac infobase drop` без
`--drop-database`, затем своя БД переименовывается во временное имя
(`ALTER DATABASE … RENAME`). Создание идёт обычным порядком; удаляется (`DROP DATABASE`) только
отложенная БД своего PostgreSQL. Чужие БД того же сервера и СУБД, которую раннер не поднимал,
команда не трогает.
