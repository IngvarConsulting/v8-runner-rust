---
id: CTR.WIRE.INFOBASE-CONFIGURATION-EXPORT-DATA
status: active
governs: product
version: 2
decision: DEC.2026-09-14.A-PROVIDER-IS-NAMED-BY-WHO-EXECUTES
artifact: docs/schemas/command-data/infobase-configuration-export.schema.json
producer: src/domain/infobase_export.rs
consumers: [cli, unica]
check: [src/command_data.rs::generated_command_data_schemas_are_current, tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it]
scope: [wire, cli]
---

# `data` команды `infobase configuration export`

Выгрузка пакета конфигурации из базы отчитывается не только результатом, но и выбором
исполнителя: `selection` называет выбранного провайдера, причину выбора и каждого
рассмотренного кандидата с его готовностью и уликой этой готовности. Без этого отказ
«среда недоступна» неотличим от «провайдер не реализован», а чинить их надо по-разному.

`target_state` говорит о состоянии базы после операции: выгрузка не меняет базу, и форма
это утверждает явно.

## Пример

```json
{
  "mode": "preview",
  "provider_dispatched": false,
  "state": "working",
  "subject": {
    "kind": "main"
  },
  "selection": {
    "provider": null,
    "reason": "designer: file infobase is not ready: 'build/ib/1Cv8.1CD' is missing or is not a file",
    "candidates": [
      {
        "provider": "designer",
        "implementation": "implemented",
        "readiness": "unavailable",
        "evidence": "argv_tested",
        "reason": "file infobase is not ready: 'build/ib/1Cv8.1CD' is missing or is not a file"
      }
    ]
  },
  "artifact_kind": "cf",
  "output": "build/main.cf",
  "published": false,
  "target_state": "unchanged",
  "execution": {
    "status": "failed",
    "errors": [
      {
        "code": "environment_unavailable",
        "message": "environment unavailable: file infobase is not ready"
      }
    ]
  }
}
```
