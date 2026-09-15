---
id: CTR.WIRE.PUBLISH-DATA
status: active
governs: product
version: 2
decision: DEC.2026-09-14.RECEIPT-EXPLAINS-PROVIDER-CHOICE
artifact: docs/schemas/command-data/publish.schema.json
producer: src/domain/publish.rs
consumers: [cli, unica]
check: [src/command_data.rs::generated_command_data_schemas_are_current, tests/cli_publish.rs::publish_composes_webinst_from_the_declared_web_section]
scope: [wire, cli]
---

# `data` команды `publish`

Публикация и её удаление отчитываются тем, что было объявлено в `infobase.web` и что из
этого сделано: сервер, виртуальный и физический каталоги, клиентский адрес, если он
объявлен, и `action` — `publish` или `delete`. Превью несёт `plan` с программой и
аргументами `webinst`: параметры публикации берутся из файла, и превью показывает их
целиком, потому что публикация замещает `default.vrd` без остатка.

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
