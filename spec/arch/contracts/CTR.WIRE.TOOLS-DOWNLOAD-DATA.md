---
id: CTR.WIRE.TOOLS-DOWNLOAD-DATA
status: active
governs: product
version: 1
decision: DEC.2026-09-14.EVERY-COMMAND-PINS-THE-FORM-OF-ITS-DATA
artifact: docs/schemas/command-data/tools-download.schema.json
producer: src/domain/tools_download.rs
consumers: [cli]
check: [src/command_data.rs::generated_command_data_schemas_are_current]
scope: [wire, cli]
---

# `data` команды `tools download`

Форма называет каждый скачанный артефакт вместе с тегом выпуска, из которого он взят, и
ключом конфига, куда записан путь. Тег важнее пути: по нему видно, ту ли версию
инструмента получил проект, а путь машинно-локален и в репозиторий не едет.

Живой проверки у формы пока нет: команда ходит в сеть. Форму держит сверка с типом.

## Пример

```json
{
  "ok": true,
  "tool": "yaxunit",
  "mode": "install",
  "destinations": [
    {
      "tool": "yaxunit",
      "tag": "24.12",
      "source": "github-release",
      "path": "tools/yaxunit/YaXUnit.cfe",
      "config": "tools.yaxunit.path"
    }
  ],
  "config_path": "v8project.yaml",
  "local_config_path": "v8project.local.yaml",
  "duration_ms": 8421
}
```
