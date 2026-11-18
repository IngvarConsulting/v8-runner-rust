---
id: CTR.WIRE.DEBUG-SERVER-STOP-DATA
version: 1
artifact: docs/schemas/command-data/debug-server-stop.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it
  - src/use_cases/managed_stand/debug_server.rs::stop_stops_the_server_of_its_session_and_repeats_safely
  - src/use_cases/managed_stand/debug_server.rs::a_wrong_session_leaves_the_server_alone
  - src/use_cases/managed_stand/debug_server.rs::a_stale_session_leaves_a_process_that_took_its_pid_alone
  - tests/cli_debug_server.rs::stop_answers_while_another_command_holds_the_work_path
---

# `data` команды `debug-server stop`

`action`: `stopped`, `nothing_to_stop` или `planned` в превью; `session` — сеанс из вызова в нижнем регистре
(верхний регистр в вызове — тот же сеанс; иное, чем 32 шестнадцатеричных знака, отвергает разбор ключей). `known_session: false`
— этой рабочей копии сеанс неизвестен: он уже остановлен, или его запустили из другой рабочей копии; раннер ничего
не трогает. Повторный `stop` того же сеанса отвечает так же. `stopped_pid` — остановленный `dbgs`. `stale_record`
говорит, что запись была, а процесса из неё уже нет: сервер вышел сам, например с концом владельца, или его pid занял
другой процесс, которого раннер не трогает. У сеанса сервера стенда (`owner: cluster` или `owner: standalone`)
`action` — `nothing_to_stop`: сервер отладки живёт со своим кластером или автономным сервером, `stop` снимает
только сеанс. `url` и `port` у такого сеанса нет: порты стенда могли смениться, его живой адрес называет
`cluster status` или `standalone status`.

## Пример

```json
{
  "ok": true,
  "action": "stopped",
  "session": "5f0c2a9d4e3b4c1f8a7d6e5c4b3a2f10",
  "owner": "runner",
  "url": "http://127.0.0.1:12961",
  "port": 12961,
  "known_session": true,
  "stopped_pid": 14820,
  "duration_ms": 310
}
```
