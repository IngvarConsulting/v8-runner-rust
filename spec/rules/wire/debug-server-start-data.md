---
id: CTR.WIRE.DEBUG-SERVER-START-DATA
version: 2
artifact: docs/schemas/command-data/debug-server-start.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it
  - tests/cli_debug_server.rs::start_runs_the_debug_server_beside_the_client_and_stop_ends_it
  - src/use_cases/managed_stand/debug_server.rs::start_answers_with_the_address_that_answered_as_a_debug_server
  - src/use_cases/managed_stand/debug_server.rs::a_cluster_infobase_gets_the_debug_server_of_the_cluster
  - src/use_cases/managed_stand/debug_server.rs::a_standalone_infobase_gets_the_debug_server_of_the_server
  - src/use_cases/managed_stand/standalone.rs::the_debug_server_of_the_standalone_server_needs_a_debug_start
  - src/use_cases/managed_stand/debug_server.rs::a_preview_starts_and_writes_nothing
  - src/use_cases/managed_stand/debug_server.rs::ensure_reuses_and_recovers_its_own_debug_server
  - src/use_cases/managed_stand/debug_server.rs::ensure_cleans_up_a_new_server_without_a_verified_connection
  - src/use_cases/managed_stand/debug_server.rs::ensure_keeps_unidentified_attempts_instead_of_starting_duplicates
  - src/use_cases/managed_stand/debug_server.rs::ensure_never_repairs_a_symlinked_record
  - src/use_cases/managed_stand/debug_server.rs::ensure_preserves_a_session_with_a_missing_process_record
  - src/use_cases/managed_stand/debug_server.rs::copying_ensured_state_does_not_grant_access_to_its_live_server
---

# `data` команды `debug-server start`

`action`: `started` при запуске, `reused` при переиспользовании через `--ensure`,
`planned` в превью. Если попытка `--ensure` с передачей работы исполнителю
завершилась ошибкой, ответ содержит `ok: false`, `action: failed` и причину
в конверте. Адрес в таком ответе отсутствует. `owner` — чей сервер отладки: `runner` — свой `dbgs` раннера для
файловой базы, `cluster` — сервер отладки управляемого кластера базы (с HTTP-отладкой, Windows и Linux),
`standalone` — `dbgs` управляемого автономного сервера базы (`standalone start --debug`, Windows).
`url` — адрес для `/DEBUGGERURL` клиента и для отладчика, `http://127.0.0.1:<порт>`; `port` — тот же порт.
`session` — сеанс для `debug-server stop`, 32 шестнадцатеричных знака; у превью его нет. `infobase_alias` — имя
базы для `infoBaseAlias` запросов `rdbg`: `DefAlias` у файловой базы, имя базы в сервере у автономного сервера; у
кластера алиас равен зарегистрированному имени базы из `Ref`.

У `owner: runner` ещё `pid` процесса `dbgs`, `owner_pid` — процесс, с концом которого сервер выходит сам
(`--owner-pid`), `platform_resolution` — путь `dbgs` и установка тонкого клиента, из которой он взят, и `log_path`
— журнал сервера. У `owner: cluster` и `owner: standalone` процесса раннера нет, и этих полей тоже; ключи `--port`
и `--owner-pid` к серверу стенда не применяются, о чём говорит `warnings` конверта.

## Автоматическое восстановление

`debug-server start --ensure --owner-pid <PID>` готовит сервер файловой базы
на Windows и macOS. Он принимает автоматически выбранный порт. Сеанс
принадлежит рабочей копии, файловой базе и живому владельцу с тем же временем
запуска. Повторный запрос возвращает его токен и PID, если подтверждены
процесс, принадлежность адреса и ответ `rdbg`. При повторном отказе проверки
раннер останавливает ранее опознанный собственный процесс, подтверждает
остановку и пробует запустить замену. Если записанный процесс уже завершился
или его PID занял другой, раннер запускает замену без сигнала этому PID.
Новый адрес возвращается только после проверки готовности. Замена получает
другой токен и свободный порт. Запросы к одному сеансу сериализуются раннером.

Повреждённая запись или недоступная идентичность процесса не дают права
остановить его или создать дубль. Обычный `start` создаёт отдельный сеанс
при каждом вызове.

## Пример

```json
{
  "ok": true,
  "action": "started",
  "session": "5f0c2a9d4e3b4c1f8a7d6e5c4b3a2f10",
  "owner": "runner",
  "url": "http://127.0.0.1:12961",
  "port": 12961,
  "pid": 14820,
  "owner_pid": 9312,
  "platform_resolution": {
    "path": "C:\\Program Files\\1cv8\\8.3.27.2342\\bin\\dbgs.exe",
    "version": "8.3.27.2342",
    "source": "default-root",
    "installation_root": "C:\\Program Files\\1cv8\\8.3.27.2342"
  },
  "log_path": "C:\\work\\debug-server\\5f0c2a9d4e3b4c1f8a7d6e5c4b3a2f10\\5f0c2a9d4e3b4c1f8a7d6e5c4b3a2f10.log",
  "duration_ms": 1240
}
```
