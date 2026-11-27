---
id: CTR.WIRE.DEBUG-SERVER-PREPARE-DATA
version: 1
artifact: docs/schemas/command-data/debug-server-prepare.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/cli_debug_server.rs::prepare_preview_reports_the_selected_route_without_creating_state
  - tests/cli_debug_server.rs::prepare_requires_owner_pid
  - tests/cli_debug_server.rs::prepare_reuses_and_releases_an_owned_debug_server_on_linux
  - src/use_cases/managed_stand/debug_server.rs::prepared_leases_release_independently_even_after_target_selection_changes
  - src/use_cases/managed_stand/debug_server.rs::prepare_cancellation_stops_only_the_unpublished_owned_attempt
  - src/use_cases/managed_stand/debug_server.rs::prepare_uses_client_only_local_debug_for_external_cluster_without_an_address
  - src/use_cases/managed_stand/debug_server.rs::prepare_accepts_owned_proof_without_enabling_legacy_ensure
  - src/use_cases/managed_stand/debug_server.rs::prepare_borrows_exact_explicit_file_endpoint_without_a_lease
  - src/use_cases/managed_stand/cluster.rs::external_prepare_borrows_only_the_exact_live_managed_cluster_registration
  - src/config/loader.rs::a_project_file_cannot_select_an_external_debugger
  - src/config/schema.rs::native_debugger_endpoint_is_only_in_the_local_schema
  - tests/cli_debug_server.rs::prepare_refuses_an_explicit_external_cluster_debugger_without_falling_back
---

# `data` команды `debug-server prepare`

Команда готовит адрес для отладчика и клиента выбранной ИБ. `--owner-pid`
обязателен. Превью возвращает `action: planned` без действующего адреса и аренды,
не обращается к сети и не создаёт процессы или записи.

У результата `action: prepared` есть `url`, `infobase_alias`, `scope`, `owner` и
`release`. Для собственного `dbgs` также есть `owner_pid` и отдельная аренда
`release: {action: stop, session: …}`. У заимствованного сервера `release: null`.
Раннер не управляет жизненным циклом такого процесса.

Подтверждённый сервер кластера возвращает `client_and_server` с алиасом выбранной
регистрации. Для явно заданного адреса раннер подтверждает тот же локальный
кластер по соседнему управляемому профилю этой рабочей копии. Сверяются
прямой адрес и Ref, заданный `cluster.ras`, порт `dbgs`, поколения процессов,
UUID кластера и UUID регистрации.
Такой профиль получает `owner: external` и `release: null`. У файловой ИБ `client_and_server` включает ServerEmulation.
Для внешней серверной ИБ без настроенного адреса собственный loopback-сервер
раннера даёт `client`. На Linux для запуска и повторной аренды своего `dbgs`
обязательно подтверждение через `/proc` владельца IPv4 loopback-слушателя
и принятого RDBG-соединения. Подтверждение привязки к управляемому кластеру
поддержано на Windows. У явно указанного недоступного адреса код
`debug_server_external_unavailable`, у ответившего сервера без подтверждённой
связи с кластерной целью — `debug_server_external_target_unverified`. Эти отказы завершают
вызов до локальной подготовки.

Внешний адрес выбирает владелец ИБ в `v8project.local.yaml` в
`infobases.<имя>.debug.url`. Ключ `infobase-alias` задаёт проверенный алиас.
Проектному файлу запрещено передавать адрес отдельно от локального профиля.

## Пример

```json
{
  "action": "prepared",
  "url": "http://127.0.0.1:12961",
  "infobase_alias": "DevBase",
  "scope": "client_and_server",
  "owner": "cluster",
  "release": null
}
```
