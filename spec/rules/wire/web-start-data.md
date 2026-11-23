---
id: CTR.WIRE.WEB-START-DATA
version: 1
artifact: docs/schemas/command-data/web-start.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - src/use_cases/managed_stand/web.rs::web_answers_take_the_forms_of_their_commands
  - src/use_cases/managed_stand/web.rs::a_taken_port_moves_apache_and_the_answer_names_the_previous_one
---

# `data` команды `web start`

Ответ называет базу, состояние, действие, готовность, путь к частному конфигу
и журналу. Успешный фактический запуск подтверждает готовность Apache и сервера HTTP-отладки кластера.
Для своего процесса он добавляет PID, адрес и явность запуска.
При переносе порта `start` и `restart` называют прежний порт в `previous_port`.
`publication_stale` указывает, что требуется повторный `publish`.

## Пример

```json
{
  "ok": true,
  "infobase": "origin",
  "state": "not_running",
  "action": "planned",
  "ready": false,
  "config_path": ".local/infobases/origin/managed/web/httpd.conf",
  "log_path": ".local/infobases/origin/managed/web/web.log",
  "duration_ms": 0
}
```
