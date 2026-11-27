---
id: CTR.WIRE.DEBUG-SERVER-STOP-DATA
version: 4
artifact: docs/schemas/command-data/debug-server-stop.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it
  - tests/cli_debug_server.rs::stop_answers_while_another_command_holds_the_work_path
  - src/use_cases/managed_stand/debug_server.rs::prepared_leases_release_independently_even_after_target_selection_changes
  - src/use_cases/managed_stand/debug_server.rs::prepared_stop_does_not_signal_a_recycled_pid
  - src/use_cases/managed_stand/debug_server.rs::prepared_stop_waits_for_death_after_identity_becomes_unavailable
  - src/use_cases/managed_stand/debug_server.rs::copied_prepared_scope_cannot_stop_the_original_debug_server
  - src/use_cases/managed_stand/debug_server.rs::unresolved_prepared_scope_cannot_confirm_absence_of_a_lease
---

# `data` команды `debug-server stop`

`session` — токен аренды из вызова в нижнем регистре, 32 шестнадцатеричных знака.
Аренды выдаются в `release.session` ответа `debug-server start`.

- `released` снимает переданную аренду. `known_session` равен `true`. Процесс
  обслуживает другие аренды, `stopped_pid` отсутствует.
- `stopped` подтверждает завершение собственного процесса по последней аренде
  и возвращает его `stopped_pid`.
- `nothing_to_stop` — токен уже освобождён или неизвестен этой рабочей копии.
  У неизвестного токена `known_session: false`. `stale_record` сообщает, что
  запись относилась к завершившемуся процессу или к прежнему владельцу PID.
- `planned` — результат превью без освобождения аренды.

## Пример

```json
{
  "ok": true,
  "action": "stopped",
  "session": "5f0c2a9d4e3b8c1f8a7d6e5c4b3a2f10",
  "owner": "runner",
  "known_session": true,
  "stopped_pid": 14820,
  "duration_ms": 310
}
```
