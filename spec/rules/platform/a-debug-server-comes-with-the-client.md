---
id: INV.PLATFORM.A-DEBUG-SERVER-COMES-WITH-THE-CLIENT
check:
  - src/platform/dbgs.rs::the_server_lies_beside_the_client
  - tests/cli_debug_server.rs::start_runs_the_debug_server_beside_the_client_and_stop_ends_it
  - src/use_cases/managed_stand/debug_server.rs::a_missing_debug_server_is_an_environment_refusal_naming_the_installation
---

# Сервер отладки берётся из установки клиента

Свой `dbgs` сеанса `debug-server` раннер запускает из той установки, из которой запускается тонкий клиент — так же, как его выбирает
`launch thin`: сервер и клиент одной версии. Нет `dbgs` рядом с клиентом — отказ `environment_unavailable` слоя
`platform`, называющий версию и путь клиента; процесс не запускается.

`dbgs` автономного сервера (`standalone start --debug`) берётся из установки его `ibsrv`: нет его рядом — отказ
`environment_unavailable` слоя `platform` с путём `ibsrv`, сервер не остаётся работать.
