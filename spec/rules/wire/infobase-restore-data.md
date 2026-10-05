---
id: CTR.WIRE.INFOBASE-RESTORE-DATA
version: 6
artifact: docs/schemas/command-data/infobase-restore.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it
  - src/use_cases/infobase_export.rs::a_restore_refused_before_any_work_leaves_the_target_unchanged
---

# `data` команды `infobase restore`

Подъём базы из снимка — единственная команда этой тройки, которая базу меняет, и форма
говорит об этом двумя разными полями: `target_mode` — что просили сделать с целью
(создать или заместить), `restored` — сделано ли. `target_state` остаётся `unchanged`,
пока платформа не отработала: превью цель не трогает.

**Что изменила версия 5.** Значение `export_or_publication` ушло из общего набора фаз; эта
форма его не давала. Каждая остановка на безопасной точке теперь пишет прерывание: статус
`cancelled` и запись с фазой `command_boundary`, а не `failed` без записи. Процесс, которому
отмена не дала запуститься, тоже называется `command_boundary`, а не `provider_command`:
работы он не получил. Отказ исполнителя, так и не получившего работу, базу не трогал:
`target_state` остаётся `unchanged`, и предупреждения о неудавшемся откате в ответе нет.
Имена шагов в `steps[]` прежние.

**Что изменила версия 6.** Причина пропуска исполнителя одна — почему он не взят:
приставка о реализованности адаптера (`Designer DT restore is implemented …; `) ушла из
`provider.skipped[].reason`, а с ней из `execution.errors[].message`, `steps[].message` и
`error.message`. Исполнитель вне матрицы, названный ключом `providers.*`, до выбора не
доходит: команда отказывает при загрузке настроек родом `invalid_argument`, как `push`.
Отказ запроса до выбора исполнителя квитанции не несёт — пример показывает именно это.

## Пример

```json
{
  "mode": "preview",
  "provider_dispatched": false,
  "subject": {
    "kind": "infobase"
  },
  "artifact_kind": "dt",
  "input": "/home/dev/project/build/main.dt",
  "target_mode": "replace",
  "restored": false,
  "target_state": "unchanged",
  "execution": {
    "status": "failed",
    "errors": [
      {
        "code": "invalid_argument",
        "message": "infobase restore requires exactly one of --create or --replace"
      }
    ]
  }
}
```
