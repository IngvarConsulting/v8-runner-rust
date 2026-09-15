---
id: CTR.WIRE.LAUNCH-DATA
status: active
governs: product
version: 2
decision: DEC.2026-09-14.LAUNCH-OPENS-THE-PUBLISHED-BASE
artifact: docs/schemas/command-data/launch.schema.json
producer: src/domain/launch.rs
consumers: [cli, mcp, unica]
check: [src/command_data.rs::generated_command_data_schemas_are_current, tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it]
scope: [wire, cli, mcp]
---

# `data` команды `launch`

Запуск клиента отчитывается тем, что именно будет запущено: программа и полный список
аргументов в `plan`, найденный исполняемый файл и то, откуда взялась платформа
(`platform_resolution.source`). Секреты в `plan` не попадают — это общее правило вывода, а
не свойство этой команды.

`pid` есть только у настоящего запуска; у превью он пуст, и `provider_dispatched: false`
говорит о том же вторым полем — вызывающему не приходится выводить факт запуска из
отсутствия значения.

## Пример

```json
{
  "ok": true,
  "mode": "designer",
  "pid": null,
  "binary": "/opt/1cv8/bin/1cv8",
  "platform_resolution": {
    "path": "/opt/1cv8/bin/1cv8",
    "version": null,
    "source": "explicit",
    "installation_root": "/opt/1cv8"
  },
  "provider_dispatched": false,
  "plan": {
    "program": "/opt/1cv8/bin/1cv8",
    "args": [
      "DESIGNER",
      "/DisableStartupDialogs",
      "/IBConnectionString",
      "File=build/ib"
    ]
  },
  "message": "Previewed конфигуратор via /opt/1cv8/bin/1cv8; client process not dispatched"
}
```
