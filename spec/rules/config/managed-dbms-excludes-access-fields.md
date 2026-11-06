---
id: INV.CONFIG.MANAGED-DBMS-EXCLUDES-ACCESS-FIELDS
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/536
---

# Объявление `dbms.managed` исключает реквизиты доступа

Рядом с `dbms.managed` остаются `dbms.name` и `dbms.locale`. Ключи `kind`, `server`, `user` и `password` — отказ
валидации с именем ключа: адрес и пароль такой СУБД выбирает раннер. Значение `dbms.managed` — пустая
карта, и пустое значение ключа означает то же самое. Каталог сборки PostgreSQL для 1С называет `tools.postgres.path`.
