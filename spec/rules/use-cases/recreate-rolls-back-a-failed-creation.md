---
id: INV.USE-CASES.RECREATE-ROLLS-BACK-A-FAILED-CREATION
check:
  - tests/cli_infobase_recreate.rs::a_failed_recreation_puts_the_old_base_back
  - tests/cli_infobase_recreate.rs::a_leftover_of_an_interrupted_recreation_is_named
  - src/use_cases/managed_stand/cluster.rs::a_failed_managed_recreation_puts_the_database_and_the_registration_back
  - src/use_cases/managed_stand/cluster.rs::a_foreign_dbms_and_a_leftover_database_are_refused
---

# Неудачное пересоздание возвращает прежнюю базу

`infobase create --recreate` не удаляет прежнюю базу, пока новая не создана: она лежит
отложенной — записи файловой базы в каталоге `.<имя каталога>.v8-runner.recreate` рядом,
своя БД под именем `<dbms.name>_v8r_recreate`, регистрация в кластере раннера снята без БД.
Удалась база — отложенное удаляется. Неудача или отмена создания — откат: то, что оставило
создание, убирается, отложенное возвращается на место, регистрация ставится снова. Ответ
называет в `removed`, что возвращено (`restored`), а что осталось не на месте (`left`) и где.

Вернувшаяся база — не та, о которой помнит каталог: память о ней стёрта, и обмен с ней
начинается с `pull <SET> --force` или `push --force`
(`INV.USE-CASES.WHAT-COUNTS-AS-MEMORY-OF-THE-BASE`).

Остаток после аварийного выхода раннера — отложенный каталог или БД с временным именем —
находят и называют следующий `infobase create` и `infobase create --recreate`: каждый
отказывает до изменений и говорит, как вернуть или удалить остаток.
