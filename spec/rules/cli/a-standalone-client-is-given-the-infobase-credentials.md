---
id: INV.CLI.A-STANDALONE-CLIENT-IS-GIVEN-THE-INFOBASE-CREDENTIALS
check:
  - tests/cli_launch.rs::a_standalone_client_carries_the_infobase_credentials_by_the_direct_gate
  - tests/cli_launch.rs::a_launched_standalone_client_receives_the_real_infobase_password
  - tests/cli_launch.rs::a_standalone_web_address_carries_the_infobase_credentials
  - tests/cli_launch.rs::a_file_web_address_keeps_the_infobase_credentials
  - tests/cli_test.rs::a_test_client_goes_by_the_direct_gate_of_a_standalone_server
  - tests/cli_test.rs::a_test_client_without_the_direct_gate_goes_by_the_web_address
  - src/platform/enterprise.rs::a_web_address_carries_configured_credentials
---

# Клиент автономной цели получает реквизиты базы через `/S` и `/WS`

`infobase.user` и `infobase.password` автономной цели — пользователь базы. Конфигуратор,
тонкий клиент и клиент тестов по строке прямого шлюза получают их ключами `/N` и `/P`
сразу за адресом. При выборе `infobase.web.url` тонкий клиент получает те же реквизиты
после `/WS`. В процесс уходит настоящий пароль, превью и ответ команды маскируют его.
