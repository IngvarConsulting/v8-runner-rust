---
id: CTR.WIRE.DEBUG-SERVER-START-DATA
version: 1
artifact: docs/schemas/command-data/debug-server-start.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it
  - tests/cli_debug_server.rs::start_runs_the_debug_server_beside_the_client_and_stop_ends_it
  - src/use_cases/managed_stand/debug_server.rs::start_answers_with_the_address_that_answered_as_a_debug_server
  - src/use_cases/managed_stand/debug_server.rs::a_cluster_infobase_gets_the_debug_server_of_the_cluster
  - src/use_cases/managed_stand/debug_server.rs::a_preview_starts_and_writes_nothing
---

# `data` команды `debug-server start`

`action`: `started` или `planned` в превью. `owner` — чей сервер отладки: `runner` — свой `dbgs` раннера для
файловой базы, `cluster` — сервер отладки управляемого кластера базы (`cluster start --debug`, только Windows).
`url` — адрес для `/DEBUGGERURL` клиента и для отладчика, `http://127.0.0.1:<порт>`; `port` — тот же порт.
`session` — сеанс для `debug-server stop`, 32 шестнадцатеричных знака; у превью его нет.

У `owner: runner` ещё `pid` процесса `dbgs`, `owner_pid` — процесс, с концом которого сервер выходит сам
(`--owner-pid`), `platform_resolution` — путь `dbgs` и установка тонкого клиента, из которой он взят, и `log_path`
— журнал сервера. У `owner: cluster` процесса раннера нет, и этих полей тоже; ключи `--port` и `--owner-pid` к
серверу кластера не применяются, о чём говорит `warnings` конверта.

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
