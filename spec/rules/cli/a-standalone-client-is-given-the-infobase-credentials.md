---
id: INV.CLI.A-STANDALONE-CLIENT-IS-GIVEN-THE-INFOBASE-CREDENTIALS
check:
  - tests/cli_launch.rs::a_standalone_client_carries_the_infobase_credentials_with_a_masked_password
  - tests/cli_launch.rs::a_launched_standalone_client_receives_the_real_infobase_password
  - tests/cli_test.rs::a_test_client_goes_by_the_direct_gate_of_a_standalone_server
  - tests/cli_test.rs::a_test_client_without_the_direct_gate_goes_by_the_web_address
  - src/platform/enterprise.rs::a_web_address_carries_the_infobase_credentials
---

# Клиент автономной цели получает реквизиты базы

`infobase.user` и `infobase.password` автономной цели — пользователь базы. Конфигуратор и
тонкий клиент по строке прямого шлюза, тонкий клиент по клиентскому адресу и клиент тестов
получают их ключами `/N` и `/P` сразу за адресом. В процесс уходит настоящий пароль; план
превью и ответ команды показывают его замаскированным.
