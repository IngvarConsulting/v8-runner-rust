---
id: CTR.WIRE.PUBLISH-DATA
version: 3
artifact: docs/schemas/command-data/publish.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/cli_publish.rs::publish_composes_webinst_from_the_declared_web_section
  - tests/cli_publish.rs::publish_preview_never_echoes_the_password_of_the_connection_string
---

# `data` команды `publish`

Публикация и её удаление отчитываются тем, что было объявлено в `infobase.web` и что из
этого сделано: сервер, виртуальный и физический каталоги, клиентский адрес, если он
объявлен, и `action` — `publish` или `delete`. Превью несёт `plan` с программой и
аргументами `webinst`: параметры публикации берутся из файла, и превью показывает их
целиком, потому что публикация замещает `default.vrd` без остатка. Целиком — значит без
пропущенных параметров, а не без маскирования: `-connstr` несёт строку соединения, и
пароль в ней закрыт как везде.

## Пример

```json
{
  "provider": {"selected": "webinst", "origin": {"kind": "default"}},
  "ok": true,
  "provider_dispatched": false,
  "action": "publish",
  "server": "apache24",
  "wsdir": "demo",
  "dir": "/var/www/demo",
  "url": "http://localhost/demo",
  "plan": {
    "program": "/opt/1cv8/bin/webinst",
    "args": [
      "-publish",
      "-apache24",
      "-wsdir",
      "demo",
      "-dir",
      "/var/www/demo",
      "-connstr",
      "File=build/ib"
    ]
  },
  "duration_ms": 0,
  "message": "would publish 'demo' on apache24 via /opt/1cv8/bin/webinst; web server not touched"
}
```
