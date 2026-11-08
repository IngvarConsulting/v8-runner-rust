---
id: INV.CONFIG.MANAGED-DBMS-EXCLUDES-ACCESS-FIELDS
check:
  - src/config/validate.rs::a_managed_dbms_refuses_every_access_field_by_its_name
  - tests/cli_dbms.rs::an_access_field_next_to_managed_is_refused_by_its_name
  - tests/cli_dbms.rs::an_empty_managed_key_means_the_same_as_an_empty_map
---

# Объявление `dbms.managed` исключает реквизиты доступа

Рядом с `dbms.managed` остаются `dbms.name` и `dbms.locale`. Ключи `kind`, `server`, `user` и `password` — отказ
валидации с именем ключа: адрес и пароль такой СУБД выбирает раннер. Значение `dbms.managed` — пустая
карта, и пустое значение ключа означает то же самое. Каталог сборки PostgreSQL для 1С называет `tools.postgres.path`.
