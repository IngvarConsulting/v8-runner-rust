---
id: CTR.WIRE.LOAD-DATA
version: 3
artifact: docs/schemas/command-data/upload.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - src/use_cases/load_artifact.rs::execute_reports_cancelled_status_before_load_probe
  - src/use_cases/load_artifact.rs::execute_reports_cancelled_status_at_update_db_cfg_safe_point
  - tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it
  - tests/cli_load.rs::upload_update_failure_preserves_the_completed_load_receipt
---

# `data` команды `load`

Применение готового артефакта к базе отчитывается тремя разными вещами сразу: что
применяли (`artifact_type`, `target_kind`), чем кончилась проба совместимости
(`compatibility_state`) и запускалась ли платформа вообще (`provider_dispatched`).
Разделять их обязательно: непроверенная совместимость и проверенная несовместимость —
разные состояния, и вызывающий обязан их различать.

Вложенный `execution` несёт машинную часть исхода: статус, ошибки с кодами и полезную
нагрузку сценария.

После успешной загрузки `.cf` или `.cfe` отказ `/UpdateDBCfg` оставляет общий
`ok: false`, но `execution.payload.applied: true`: рабочая конфигурация уже
изменилась. Если платформа вернула ошибку обновления, `update_db_cfg_ran: true`;
отказ до подтверждённого результата этого вызова оставляет `false`. Эти поля
не утверждают, что конфигурация базы данных успешно обновлена.

**Что изменила версия 3.** Фаза прерывания `execution.interruptions[].phase` стала закрытым
набором значений в `snake_case`, общим для всех форм с итогом исполнения; набор перечисляет
`$defs/ExecutionInterruptionPhase` схемы. Остановка на безопасной точке — перед пробой или
перед `/UpdateDBCfg` — называется `command_boundary` вместо `update_db_cfg_safe_point`;
`apply` и `update_db_cfg` прежние. Остановка перед пробой больше не утверждает, что платформа
запускалась и пакет загружен: `provider_dispatched` и `execution.payload.applied` там `false`.
Остановка перед `/UpdateDBCfg` после отмены, пришедшей во время загрузки, называет и
отложенную отмену: запись `apply` с `deferred: true` идёт перед `command_boundary`.

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
