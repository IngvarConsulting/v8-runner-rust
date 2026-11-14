---
id: CTR.WIRE.WEB-STATUS-DATA
version: 1
artifact: docs/schemas/command-data/web-status.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/cli_web.rs::web_status_of_a_declared_cluster_writes_nothing
---

# `data` команды `web status`

Ответ называет базу, состояние, действие, готовность, путь к частному конфигу
и журналу. Для своего процесса он добавляет PID, адрес и явность запуска.
При переносе порта `start` и `restart` называют прежний порт в `previous_port`.
`publication_stale` указывает, что требуется повторный `publish`.

## Пример

```json
{
  "ok": true,
  "infobase": "origin",
  "state": "not_running",
  "action": "reported",
  "ready": false,
  "config_path": ".local/infobases/origin/managed/web/httpd.conf",
  "log_path": ".local/infobases/origin/managed/web/web.log",
  "duration_ms": 0
}
```
