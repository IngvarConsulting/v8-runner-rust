---
id: INV.CONFIG.A-MANAGED-STANDALONE-NAMES-ITS-STORAGE
check:
  - src/config/validate.rs::a_managed_standalone_names_exactly_one_storage
---

# Автономный сервер под управлением раннера называет одно хранилище

`standalone.managed` требует `name` и хранилище: `db-path` либо секцию `dbms`. Всё вместе и ничего — отказ
валидации. Секция `dbms` рядом с автономным сервером, который поднимаете вы, остаётся отказом
`DbmsNotAllowedForStandalone`: такой сервер сам знает свою базу.
