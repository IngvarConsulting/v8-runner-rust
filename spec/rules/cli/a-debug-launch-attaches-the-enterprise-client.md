---
id: INV.CLI.A-DEBUG-LAUNCH-ATTACHES-THE-ENTERPRISE-CLIENT
check:
  - src/platform/enterprise.rs::a_debugger_url_attaches_the_client_to_its_debug_server
  - tests/cli_launch_debug.rs::a_debug_launch_preview_attaches_the_client_to_the_debug_server
  - tests/cli_launch_debug.rs::a_debug_launch_is_refused_where_it_cannot_work
---

# `launch --debug` подключает клиент предприятия к серверу отладки

`launch thin|thick|ordinary|mcp --debug <url>` добавляет в командную строку клиента
`/DEBUG -http -attach /DEBUGGERURL <url>` перед ключами `--raw-key`: клиент подключается к серверу отладки
сразу после запуска, и `-attach` подключает его предметы отладки без выбора в отладчике. Адрес — тот, что
ответил `debug-server start`, вида `http://<хост>:<порт>`; раннер сервер перед запуском не проверяет.

Отказ `invalid_argument` без запуска: `launch designer --debug` (у Конфигуратора свой отладчик), `launch web
--debug` (веб-клиент ключей командной строки не получает), адрес не вида `http://<хост>:<порт>` и ключи
`/DEBUG` или `/DEBUGGERURL` в `--raw-key` рядом с `--debug`.
