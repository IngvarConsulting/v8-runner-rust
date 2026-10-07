---
id: INV.CLI.A-CLIENT-GOES-BY-THE-CONNECTION-STRING-BEFORE-THE-WEB-ADDRESS
check:
  - tests/cli_launch.rs::a_thin_client_keeps_the_connection_address_by_default
  - tests/cli_launch.rs::a_thin_client_goes_through_the_web_address_when_asked
  - tests/cli_launch.rs::a_standalone_thin_client_goes_by_the_direct_gate_by_default
  - tests/cli_launch.rs::a_standalone_thin_client_without_the_direct_gate_goes_by_the_web_address
  - tests/cli_launch.rs::a_user_connection_key_lands_after_the_direct_gate_address
  - tests/cli_agent_standalone.rs::a_thin_client_without_either_address_names_both
  - tests/cli_agent_standalone.rs::via_connection_without_the_direct_gate_names_the_connection_string
  - tests/cli_test.rs::a_test_client_goes_by_the_direct_gate_of_a_standalone_server
  - tests/cli_test.rs::a_test_client_without_the_direct_gate_goes_by_the_web_address
---

# Клиент идёт по строке подключения, а без неё — по клиентскому адресу

`launch thin` и `launch mcp` без `--via` открывают базу по `infobase.connection`, а когда
строка не объявлена — по `infobase.web.url` как ws-соединение. Объявленный клиентский адрес
строку не подменяет: путь выбирает наличие строки, а не наличие публикации. Правило одно для
всех видов цели; у автономного сервера строка подключения — строка прямого шлюза, и клиент
идёт по ней ключом `/S <host>:<port>\<name>`, как в кластер. Клиент тестов (`test`) выбирает
адрес тем же правилом.

`--via web` и `--via connection` выбирают адрес явно. Автономная цель без строки прямого
шлюза на `--via connection` отказывает ошибкой валидации, которая называет строку; без строки
и без `infobase.web.url` тонкому клиенту идти некуда, и отказ называет и строку, и
`infobase.web.url`. Путь разрешается до поиска утилиты: отказ про адрес не подменяется
отказом про платформу.

Известный предел: ключи соединения не зарезервированы. Пользовательский
`/IBConnectionString` из `tools.enterprise.additional-launch-keys` встаёт после нашего `/S`,
а справка платформы (`IBConnectionString`, раздел «Связи») требует, чтобы
`/IBConnectionString` стоял раньше `/S`. Раннер этот порядок не меняет; какой адрес возьмёт
платформа, решает она.
