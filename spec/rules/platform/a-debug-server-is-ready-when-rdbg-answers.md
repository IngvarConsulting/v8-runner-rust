---
id: INV.PLATFORM.A-DEBUG-SERVER-IS-READY-WHEN-RDBG-ANSWERS
check:
  - src/platform/dbgs.rs::only_the_rdbg_refusal_counts_as_an_answer
  - src/platform/dbgs.rs::the_notify_file_names_the_port_in_utf16_with_a_bom
  - tests/cli_debug_server.rs::start_runs_the_debug_server_beside_the_client_and_stop_ends_it
  - src/use_cases/managed_stand/debug_server.rs::start_answers_with_the_address_that_answered_as_a_debug_server
  - src/use_cases/managed_stand/debug_server.rs::a_readiness_timeout_stops_the_attempt
  - src/use_cases/managed_stand/debug_server.rs::an_exit_at_once_leaves_no_session
  - src/use_cases/managed_stand/debug_server.rs::an_exit_before_readiness_leaves_no_session
  - src/use_cases/managed_stand/debug_server.rs::a_cancellation_stops_the_attempt
  - src/use_cases/managed_stand/debug_server.rs::an_answer_from_another_process_is_not_readiness
  - src/use_cases/managed_stand/cluster.rs::the_debug_port_of_the_cluster_needs_a_debug_start
  - src/use_cases/managed_stand/standalone.rs::a_debug_server_that_does_not_start_stops_the_server
---

# Сервер отладки готов, когда отвечает по `rdbg`

`debug-server start` отвечает успехом, только когда сервер назвал свой адрес и по этому адресу ответил обработчик
протокола `rdbg`: на `POST /e1crdbg/rdbg?cmd=getDbgTargets` с `Content-Type: application/xml` без зарегистрированного
отладчика приходит 400 с исключением `virtual-resource-system` — в JSON при `Accept: application/json`, в XML без
него. Запущенный процесс, открытый порт или любой другой
ответ HTTP готовности не подтверждают; на Windows порт ещё должен слушать сам запущенный процесс, а не чужой. Свой `dbgs` называет адрес в файле `--notify`; у сервера кластера адрес —
закреплённый порт отладки, а проба ждёт его: агент поднимает `dbgs` лениво. У автономного сервера адрес — порт
его `dbgs` по записи; сервер не работает, поднят без отладки или его `dbgs` нет — отказ сразу, без ожидания. Процесс вышел, срок готовности прошёл
или команду отменили — процесс попытки снимается вместе с записью сеанса, и успеха нет. Журнал попытки
остаётся рядом, `workPath/debug-server/failed-<сеанс>.log`, и отказ называет его путь. Если процесс не погас,
отказ говорит это и называет сеанс: его запись остаётся для `debug-server stop`.
