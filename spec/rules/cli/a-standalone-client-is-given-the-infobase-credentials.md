---
id: INV.CLI.A-STANDALONE-CLIENT-IS-GIVEN-THE-INFOBASE-CREDENTIALS
check:
  - tests/cli_launch.rs::a_standalone_client_carries_the_infobase_credentials_by_the_direct_gate
  - tests/cli_launch.rs::a_launched_standalone_client_receives_the_real_infobase_password
  - tests/cli_launch.rs::a_standalone_web_address_carries_no_credentials
  - tests/cli_launch.rs::a_file_web_address_keeps_the_infobase_credentials
  - tests/cli_test.rs::a_test_client_goes_by_the_direct_gate_of_a_standalone_server
  - tests/cli_test.rs::a_test_client_without_the_direct_gate_goes_by_the_web_address
  - src/platform/enterprise.rs::a_web_address_carries_the_credentials_only_when_decided
---

# Клиент автономной цели получает реквизиты базы по строке прямого шлюза

`infobase.user` и `infobase.password` автономной цели — пользователь базы. Конфигуратор,
тонкий клиент и клиент тестов, идущие по строке прямого шлюза, получают их ключами `/N` и
`/P` сразу за адресом. В процесс уходит настоящий пароль; план превью и ответ команды
показывают его замаскированным.

По клиентскому адресу `infobase.web.url` автономной цели реквизиты не идут: ни `/N`, ни
`/P` в командной строке клиента нет. Их приём по `/WS` автономного сервера не замерен
(`INV.PLATFORM.A-STANDALONE-CLIENT-ACCEPTS-THE-INFOBASE-CREDENTIALS`). У файловой и
кластерной цели клиентский адрес несёт реквизиты базы.
