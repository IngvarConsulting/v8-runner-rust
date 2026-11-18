---
id: INV.CLI.A-DEBUG-LAUNCH-ATTACHES-THE-ENTERPRISE-CLIENT
check:
  - src/platform/enterprise.rs::a_debugger_url_attaches_the_client_to_its_debug_server
  - tests/cli_launch_debug.rs::a_debug_launch_preview_attaches_the_client_to_the_debug_server
  - tests/cli_launch_debug.rs::a_debug_launch_is_refused_where_it_cannot_work
  - tests/cli_launch_debug.rs::a_debugger_in_the_configured_launch_keys_is_refused_next_to_debug
  - src/domain/launch.rs::a_debugger_url_is_http_host_and_port_only
---

# `launch --debug` подключает клиент предприятия к серверу отладки

`launch thin|thick|ordinary|mcp --debug <url>` добавляет в командную строку клиента
`/DEBUG -http -attach /DEBUGGERURL <url>` перед ключами `--raw-key`: клиент подключается к серверу отладки
сразу после запуска, и `-attach` подключает его предметы отладки без выбора в отладчике. Адрес — тот, что
ответил `debug-server start`, вида `http://<хост>:<порт>`; раннер сервер перед запуском не проверяет.

Отказ `invalid_argument` без запуска: `launch designer --debug` (у Конфигуратора свой отладчик), `launch web
--debug` (веб-клиент ключей командной строки не получает), адрес не вида `http://<хост>:<порт>` (другая схема, нет
порта или порт 0, путь, параметры, фрагмент, учётные данные; порт 80 и IPv6 в скобках — адреса) и ключи `/DEBUG`
или `/DEBUGGERURL` в `--raw-key` или в `tools.enterprise.additional-launch-keys` рядом с `--debug`: второй сервер
отладки клиенту не передаётся. Отказ не повторяет введённый адрес — в нём могут быть учётные данные.
