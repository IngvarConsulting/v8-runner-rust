---
id: INV.USE-CASES.A-CONNECTION-STRING-BASE-OBEYS-OWNERSHIP-WITHOUT-OWNING
check:
  - tests/cli_infobase_owner.rs::a_connection_string_obeys_the_owner_and_never_owns
  - src/use_cases/infobase_owner.rs::only_a_run_of_a_write_on_a_declared_base_records_a_copy
  - tests/cli_infobase_owner.rs::a_connection_string_on_a_shared_base_is_refused
---

# Строка соединения подчиняется владельцу, но им не становится

Команда записи со строкой соединения в `--infobase` на файловой базе другой рабочей копии
отказывает так же, как с именем базы; свою копию в метку она не записывает ни на какой базе,
в том числе без метки или с ушедшим владельцем. Согласия делить базу строка соединения не даёт: на общей
базе (`INV.USE-CASES.A-BASE-IS-SHARED-BY-CONSENT-OF-EVERY-COPY`) команда записи с ней тоже
отказывает, и отказ просит имя базы, объявленной в местном слое с `shared: true`. Решение
владельца от 06.10.2026. Команды записи и чтения определены в
`INV.USE-CASES.A-DEVELOPMENT-BASE-IS-HELD-BY-ONE-WORKING-COPY`.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).
