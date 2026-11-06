---
id: INV.CONFIG.A-MANAGED-STANDALONE-NAMES-ITS-STORAGE
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/537
---

# Автономный сервер под управлением раннера называет одно хранилище

`standalone.managed` требует `name` и хранилище: `db-path` либо секцию `dbms`. Всё вместе и ничего — отказ
валидации. Секция `dbms` рядом с автономным сервером, который поднимаете вы, остаётся отказом
`DbmsNotAllowedForStandalone`: такой сервер сам знает свою базу.
