---
id: CTR.WIRE.LOAD-DATA
version: 4
artifact: docs/schemas/command-data/upload.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - src/use_cases/load_artifact.rs::execute_reports_cancelled_status_before_load_probe
  - src/use_cases/load_artifact.rs::execute_reports_cancelled_status_at_update_db_cfg_safe_point
  - tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it
  - tests/cli_load.rs::upload_update_failure_preserves_the_completed_load_receipt
  - src/use_cases/load_artifact.rs::a_configuration_probe_cancelled_after_its_start_reports_the_work
  - src/use_cases/load_artifact.rs::an_extension_probe_cancelled_after_its_start_reports_the_work
  - src/use_cases/load_artifact.rs::a_failed_load_after_a_deferred_cancellation_still_names_it
---

# `data` команды `load`

Применение готового артефакта к базе отчитывается тремя разными вещами сразу: что
применяли (`artifact_type`, `target_kind`), чем кончилась проба совместимости
(`compatibility_state`) и получил ли исполнитель работу (`provider_dispatched`,
[общее правило](provider-dispatched-says-whether-an-executor-got-work.md)).
Разделять их обязательно: непроверенная совместимость и проверенная несовместимость —
разные состояния, и вызывающий обязан их различать.

Вложенный `execution` несёт машинную часть исхода: статус, ошибки с кодами и полезную
нагрузку сценария.

После успешной загрузки `.cf` или `.cfe` отказ `/UpdateDBCfg` оставляет общий
`ok: false`, но `execution.payload.applied: true`: рабочая конфигурация уже
изменилась. Если платформа вернула ошибку обновления, `update_db_cfg_ran: true`;
отказ до подтверждённого результата этого вызова оставляет `false`. Эти поля
не утверждают, что конфигурация базы данных успешно обновлена.

Остановка на безопасной точке — перед пробой или перед `/UpdateDBCfg` — называется
`command_boundary`. Если отмену до этого отложила загрузка, её запись `apply` с
`deferred: true` идёт перед `command_boundary`.

**Что изменила версия 4.** Значение `export_or_publication` ушло из общего набора фаз;
`upload` его не давал. Проба совместимости, снятая отменой после запуска, отвечает
`status: cancelled` с записью фазы `provider_command` и `compatibility_state: not_established`:
вопрос задан, ответа нет. Прежде такая проба отвечала `failed` с `not_probed`, а проба
расширения — отказом проверки. `not_established` теперь и у всякой другой пробы, которая
запустилась и ответа не дала. Загрузка или `/UpdateDBCfg`, пережившие отложенную отмену и
потом не удавшиеся, остаются отказом, но отложенную отмену называют: её запись с
`deferred: true` и предупреждение идут первыми.

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
