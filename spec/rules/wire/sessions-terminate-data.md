---
id: CTR.WIRE.SESSIONS-TERMINATE-DATA
version: 1
artifact: docs/schemas/command-data/sessions-terminate.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/cli_sessions.rs::terminate_all_includes_application_jobs_and_leaves_protected_sessions
  - tests/cli_sessions.rs::terminate_preserves_successful_ids_when_another_session_is_refused
---

# `data` команды `sessions terminate`

Ответ содержит `infobase`, `managed_ras`, `requested` (UUID или `all`), `terminated_session_ids` и `errors`. При частичном отказе квитанция сохраняет успешные UUID и отдельную ошибку для каждого отказавшего UUID. Массовое завершение пропускает RAS, SystemBackgroundJob и сеансы Конфигуратора, поскольку платформа не указывает владельца процесса. Обычные BackgroundJob завершаются.

`managed_ras` показывает, поднял ли раннер RAS на время команды.

## Пример

```json
{
  "infobase": "origin",
  "managed_ras": true,
  "requested": "all",
  "terminated_session_ids": [
    "11111111-1111-1111-1111-111111111111"
  ],
  "errors": []
}
```
