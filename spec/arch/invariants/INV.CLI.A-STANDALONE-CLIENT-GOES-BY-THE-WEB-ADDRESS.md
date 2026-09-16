---
id: INV.CLI.A-STANDALONE-CLIENT-GOES-BY-THE-WEB-ADDRESS
status: active
governs: product
decision: DEC.2026-09-16.A-THIN-CLIENT-OPENS-EITHER-ADDRESS
check: [tests/cli_agent_standalone.rs::a_thin_client_against_a_standalone_server_asks_for_the_web_address, tests/cli_launch.rs::a_thin_client_goes_through_the_web_address_when_asked]
scope: [cli]
---

# Тонкий клиент к автономному серверу идёт по клиентскому адресу

Умолчание для автономной цели — веб: `launch thin` без всякого ключа открывает базу по
`infobase.web.url` как ws-соединение. Адрес не объявлен — отказ называет именно его, а не
платформу: путь разрешается до поиска утилиты.
