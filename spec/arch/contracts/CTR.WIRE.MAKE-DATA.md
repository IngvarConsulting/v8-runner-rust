---
id: CTR.WIRE.MAKE-DATA
status: active
governs: product
version: 2
decision: DEC.2026-09-14.RECEIPT-EXPLAINS-PROVIDER-CHOICE
artifact: docs/schemas/command-data/make.schema.json
producer: src/cli/execute.rs
consumers: [cli, unica]
check: [src/command_data.rs::generated_command_data_schemas_are_current, tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it]
scope: [wire, cli]
---

# `data` команды `make`

Сборка поставляемого артефакта отчитывается его видом, путём и — во вложенном
`execution.payload` — именами файлов, которые легли на диск. Имена нужны отдельно от пути:
у поставки из нескольких файлов путь один, а файлов много.

`published: false` при успешном исполнении означает превью: артефакт спланирован, но не
выложен.

## Пример

```json
{
  "provider": {"selected": "designer", "origin": {"kind": "default"}},
  "ok": true,
  "provider_dispatched": false,
  "mode": "configuration_cf",
  "source_set": "main",
  "output_path": "build/main.cf",
  "duration_ms": 0,
  "message": "would build ConfigurationCf into 'build/main.cf' via /opt/1cv8/bin/1cv8; nothing published",
  "execution": {
    "status": "succeeded",
    "diagnostics": [
      "would build ConfigurationCf into 'build/main.cf'; nothing published"
    ],
    "payload": {
      "artifact_type": "configuration_cf",
      "output_path": "build/main.cf",
      "file_names": [
        "main.cf"
      ],
      "published": false
    }
  }
}
```
