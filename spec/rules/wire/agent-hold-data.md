---
id: CTR.WIRE.AGENT-HOLD-DATA
version: 1
artifact: docs/schemas/command-data/agent-hold.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/cli_live_agent_portable.rs::agent_hold_is_idempotent_and_release_removes_the_mark
---

# `data` команды `agent hold`

`key` — ключ живого агента цели проекта: шестнадцатеричная строка, без частей, из которых он
сложен. `reused` — взят ли уже живой агент (`true`) или `hold` поднял его сам (`false`).
`holders` — держатели после `hold`, в том виде, что и в квитанции
([правило](a-receipt-says-whether-the-agent-was-live.md)): процесс — `pid` и `started_at`, срок —
`until`. `keeper_pid` — процесс сторожа, `agent_pid` — процесс агента, если платформа его
назвала.

## Пример

```json
{
  "key": "5f0c2a9e41d7b3c86a1e0f4d2b7c9a31",
  "reused": false,
  "holders": [
    { "pid": 4120, "started_at": "2026-11-10T08:12:44Z" }
  ],
  "keeper_pid": 9311,
  "agent_pid": 9340,
  "duration_ms": 6420
}
```
