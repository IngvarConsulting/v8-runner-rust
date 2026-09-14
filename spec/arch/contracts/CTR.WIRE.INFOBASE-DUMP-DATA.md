---
id: CTR.WIRE.INFOBASE-DUMP-DATA
status: active
governs: product
version: 2
decision: DEC.2026-09-14.A-PROVIDER-IS-NAMED-BY-WHO-EXECUTES
artifact: docs/schemas/command-data/infobase-dump.schema.json
producer: src/domain/infobase_export.rs
consumers: [cli, unica]
check: [src/command_data.rs::generated_command_data_schemas_are_current, tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it]
scope: [wire, cli]
---

# `data` команды `infobase dump`

Снимок базы целиком (`.dt`) отчитывается той же логикой выбора провайдера, что и выгрузка
пакета, но своим предметом: `subject.kind` здесь — вся база, а не конфигурация внутри неё.
Формы разведены, потому что состав полей у них расходится и будет расходиться дальше.

## Пример

```json
{
  "mode": "preview",
  "provider_dispatched": false,
  "subject": {
    "kind": "infobase"
  },
  "selection": {
    "provider": null,
    "reason": "ibcmd: IBCMD DT export is disabled until an exclusive-access preflight is implemented",
    "candidates": [
      {
        "provider": "ibcmd",
        "implementation": "experimental",
        "readiness": "not_checked",
        "evidence": "documented",
        "reason": "IBCMD DT export is disabled until an exclusive-access preflight is implemented"
      }
    ]
  },
  "artifact_kind": "dt",
  "output": "build/main.dt",
  "published": false,
  "target_state": "unchanged",
  "execution": {
    "status": "failed",
    "errors": [
      {
        "code": "environment_unavailable",
        "message": "environment unavailable: no provider is ready"
      }
    ]
  }
}
```
