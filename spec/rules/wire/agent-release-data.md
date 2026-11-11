---
id: CTR.WIRE.AGENT-RELEASE-DATA
version: 2
artifact: docs/schemas/command-data/agent-release.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/cli_live_agent_portable.rs::agent_hold_is_idempotent_and_release_removes_the_mark
  - tests/cli_live_agent_portable.rs::agent_release_disconnects_the_live_agent_from_the_infobase
---

# `data` команды `agent release`

`key` — ключ живого агента цели проекта; `null`, если живого агента нет. `released` — была
ли отметка этого держателя и снята ли она: `false` значит, что снимать было нечего, и это не
отказ. `holders` — держатели, оставшиеся после `release`. `disconnected` — отключил ли этот
`release` агента от базы (`INV.PLATFORM.A-LIVE-AGENT-STAYS-CONNECTED-BETWEEN-COMMANDS`):
`false` — агент не был подключён или занят командой.

## Пример

```json
{
  "key": "5f0c2a9e41d7b3c86a1e0f4d2b7c9a31",
  "released": true,
  "holders": [],
  "disconnected": true,
  "duration_ms": 12
}
```
