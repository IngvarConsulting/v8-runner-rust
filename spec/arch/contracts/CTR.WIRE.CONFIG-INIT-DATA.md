---
id: CTR.WIRE.CONFIG-INIT-DATA
status: active
governs: product
version: 2
decision: DEC.2026-09-14.BUILDER-KEY-IS-REMOVED
artifact: docs/schemas/command-data/init.schema.json
producer: src/domain/config_init.rs
consumers: [cli]
check: [src/command_data.rs::generated_command_data_schemas_are_current, tests/cli_config_init.rs::config_init_uses_json_envelope_and_output_override]
scope: [wire, cli]
---

# `data` команды `config init`

Форма отвечает на один вопрос: что записано в только что созданный конфиг. Найденные
наборы исходников перечислены тем же составом полей, каким они лягут в `v8project.yaml`,
а `overwritten` говорит, был ли затёрт существовавший файл.

Живой проверки у формы пока нет: команда пишет файлы в каталог вызова, и прогон её в
харнессе требует отдельного рабочего каталога. Форму держит сверка с типом.

## Пример

```json
{
  "ok": true,
  "path": "v8project.yaml",
  "local_path": "v8project.local.yaml",
  "gitignore_path": ".gitignore",
  "format": "DESIGNER",
  "platform_version": "8.3.27.2074",
  "source_sets": [
    {
      "name": "main",
      "type": "CONFIGURATION",
      "path": "src/cf"
    }
  ],
  "overwritten": false,
  "duration_ms": 12,
  "warnings": []
}
```
