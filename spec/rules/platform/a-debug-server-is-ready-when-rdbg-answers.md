---
id: INV.PLATFORM.A-DEBUG-SERVER-IS-READY-WHEN-RDBG-ANSWERS
check:
  - src/platform/dbgs.rs::only_the_rdbg_refusal_counts_as_an_answer
  - src/platform/dbgs.rs::the_notify_file_names_the_port_in_utf16_with_a_bom
  - tests/cli_debug_server.rs::start_runs_the_debug_server_beside_the_client_and_stop_ends_it
  - src/use_cases/managed_stand/debug_server.rs::debug_start_does_not_publish_exited_unresponsive_or_foreign_processes
  - src/use_cases/managed_stand/debug_server.rs::prepare_cancellation_stops_only_the_unpublished_owned_attempt
  - src/use_cases/managed_stand/cluster.rs::external_prepare_borrows_only_the_exact_live_managed_cluster_registration
  - src/use_cases/managed_stand/standalone.rs::a_debug_server_that_does_not_start_stops_the_server
  - src/use_cases/managed_stand/standalone.rs::a_silent_debug_server_is_stopped_with_the_server
---

# Сервер отладки готов, когда отвечает по `rdbg`

`debug-server start` отвечает успехом, только когда сервер назвал свой адрес и по этому адресу ответил обработчик
протокола `rdbg`: на `POST /e1crdbg/rdbg?cmd=getDbgTargets` с `Content-Type: application/xml` без зарегистрированного
отладчика приходит 400 с исключением `virtual-resource-system` — в JSON при `Accept: application/json`, в XML без
него. Запущенный процесс, открытый порт или любой другой
ответ HTTP готовности не подтверждают. Для собственного сервера раннер сверяет идентичность процесса
и владельца сокета вокруг RDBG-запроса. Свой `dbgs` называет порт в файле `--notify`. Адрес
управляемого кластера берётся из закреплённого порта отладки. Его принадлежность сверяется
с регистрацией выбранной базы. Готовность `dbgs` автономного сервера проверяет сам `standalone`.

Выход процесса, истечение срока или отмена до готовности прерывают запуск без выдачи аренды.
Раннер завершает собственный процесс попытки и подтверждает выход. При сбое до выдачи аренды
раннер сохраняет диагностический журнал, если процесс уже написал его. При неподтверждённом
завершении раннер сохраняет запись для сверки при следующей попытке запуска.
