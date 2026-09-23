---
id: CTR.WIRE.LOAD-DATA
version: 2
artifact: docs/schemas/command-data/upload.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it
---

# `data` команды `load`

Применение готового артефакта к базе отчитывается тремя разными вещами сразу: что
применяли (`artifact_type`, `target_kind`), чем кончилась проба совместимости
(`compatibility_state`) и запускалась ли платформа вообще (`provider_dispatched`).
Разделять их обязательно: непроверенная совместимость и проверенная несовместимость —
разные состояния, и вызывающий обязан их различать.

Вложенный `execution` несёт машинную часть исхода: статус, ошибки с кодами и полезную
нагрузку сценария.

## Пример

```json
{
  "provider": {"selected": "designer", "origin": {"kind": "default"}},
  "ok": false,
  "provider_dispatched": false,
  "mode": "load",
  "artifact_path": "build/main.cf",
  "artifact_type": "configuration_cf",
  "target_kind": "configuration",
  "compatibility_state": "not_probed",
  "duration_ms": 0,
  "message": "validation error: --path file does not exist: build/main.cf",
  "execution": {
    "status": "failed",
    "errors": [
      {
        "code": "artifact_load_failed",
        "message": "validation error: --path file does not exist: build/main.cf"
      }
    ],
    "payload": {
      "applied": false,
      "target_kind": "configuration",
      "compatibility_state": "not_probed",
      "update_db_cfg_ran": false
    }
  }
}
```
