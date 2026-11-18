---
id: INV.PLATFORM.A-DEBUG-SERVER-IS-READY-WHEN-RDBG-ANSWERS
check:
  - src/platform/dbgs.rs::only_the_rdbg_refusal_counts_as_an_answer
  - src/platform/dbgs.rs::the_notify_file_names_the_port_in_utf16_with_a_bom
  - src/use_cases/managed_stand/debug_server.rs::start_answers_with_the_address_that_answered_as_a_debug_server
  - src/use_cases/managed_stand/debug_server.rs::a_readiness_timeout_stops_the_attempt
  - src/use_cases/managed_stand/cluster.rs::the_debug_port_of_the_cluster_needs_a_debug_start
---

# Сервер отладки готов, когда отвечает по `rdbg`

`debug-server start` отвечает успехом, только когда сервер назвал свой адрес и по этому адресу ответил обработчик
протокола `rdbg`: на `POST /e1crdbg/rdbg?cmd=getDbgTargets` с `Content-Type: application/xml` без зарегистрированного
отладчика приходит 400 с исключением `virtual-resource-system`. Запущенный процесс, открытый порт или любой другой
ответ HTTP готовности не подтверждают. Свой `dbgs` называет адрес в файле `--notify`; у сервера кластера адрес —
закреплённый порт отладки, а проба ждёт его: агент поднимает `dbgs` лениво. Процесс вышел, срок готовности прошёл
или команду отменили — процесс попытки снимается вместе с записью сеанса, и успеха нет.
