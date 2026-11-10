---
id: CTR.WIRE.AGENT-STATUS-DATA
version: 1
artifact: docs/schemas/command-data/agent-status.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/cli_live_agent_portable.rs::agent_stop_removes_the_live_agent_and_status_lists_it
---

# `data` команды `agent status`

`agents` — живые агенты учётной записи ОС. У каждого: `key`; `infobase` — база в каноническом
виде (`file:…`, `server:…`; строка подключения иной формы — только отпечатком
`connection:<sha256>`, без учётных данных); `this_project` — агент цели этого проекта;
`keeper_pid` и `agent_pid`; `holders` — держатели в виде квитанции; `busy` — команда, которую
агент выполняет или выполнял последней в открытой сессии; `connected` — подключён ли он к
базе сессией команды; `idle_ms` — сколько простаивает (у занятого ноль); `closing` — сторож
уже решил его снять. Сторож, не ответивший за несколько секунд, назван по записи: без
держателей, `busy: null` и `connected: false`. Команда замка рабочего каталога не берёт и
ничего не поднимает.

## Пример

```json
{
  "agents": [
    {
      "key": "5f0c2a9e41d7b3c86a1e0f4d2b7c9a31",
      "infobase": "file:c:/ib/demo (C:\ib\demo)",
      "this_project": true,
      "keeper_pid": 9311,
      "agent_pid": 9340,
      "holders": [
        { "until": "2026-11-10T09:12:44Z" }
      ],
      "busy": "config dump-config-to-files --dir=dump/4120-1a",
      "connected": true,
      "idle_ms": 0,
      "closing": false
    }
  ],
  "duration_ms": 18
}
```
