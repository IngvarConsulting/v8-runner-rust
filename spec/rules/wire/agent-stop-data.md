---
id: CTR.WIRE.AGENT-STOP-DATA
version: 2
artifact: docs/schemas/command-data/agent-stop.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/cli_live_agent_portable.rs::agent_stop_removes_the_live_agent_and_status_lists_it
---

# `data` команды `agent stop`

`stopped` — снятые живые агенты: цели проекта, а с `--all` — все живые агенты учётной записи
ОС. Пустой список значит, что снимать было нечего. Запись агента та же, что у `agent status`
([форма](agent-status-data.md)), с состоянием на миг перед снятием. Агент с `closing: true` ещё
выполняет критическую команду: сторож принял стоп и снимет его после неё, а `agent stop` не
дождался ответа за свой срок или его отменили.

`refused` — агенты, которых `--all` снять не смог, с отказом: `key` и `message`. Обход других
агентов он не прерывает. Без `--all` отказ сторожа — отказ команды, и `refused` пуст.

## Пример

```json
{
  "stopped": [
    {
      "key": "5f0c2a9e41d7b3c86a1e0f4d2b7c9a31",
      "infobase": "file:/srv/ib/demo (/srv/ib/demo)",
      "this_project": true,
      "keeper_pid": 9311,
      "agent_pid": 9340,
      "holders": [],
      "busy": null,
      "connected": false,
      "attached": false,
      "idle_ms": 41250,
      "closing": false
    }
  ],
  "refused": [],
  "duration_ms": 830
}
```
