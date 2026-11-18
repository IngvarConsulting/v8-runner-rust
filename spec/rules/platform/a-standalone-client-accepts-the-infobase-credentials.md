---
id: INV.PLATFORM.A-STANDALONE-CLIENT-ACCEPTS-THE-INFOBASE-CREDENTIALS
check:
  - tests/cli_launch.rs::a_standalone_client_carries_the_infobase_credentials_by_the_direct_gate
  - tests/cli_launch.rs::a_standalone_web_address_carries_the_infobase_credentials
---

# Клиент автономного сервера принимает реквизиты базы

Тонкий клиент и интерактивный Конфигуратор открывают базу автономного сервера с
пользователями по строке прямого шлюза `/S <host>:<port>\<name>` с `/N` и `/P`, а тонкий
клиент принимает те же ключи при клиентском адресе `/WS`.
[Замер #184](https://github.com/IngvarConsulting/v8-runner-rust/pull/751)
