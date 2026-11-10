---
id: CTR.WIRE.INFOBASE-RESTORE-DATA
version: 9
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

Отказ исполнителя, так и не получившего работу, базу не трогал:
`target_state` остаётся `unchanged`, и предупреждения о неудавшемся откате в ответе нет.

**Что изменила версия 9.** У квитанции исполнителя в `endpoint` управляемого агента новое поле `live`: взяла ли команда уже живого агента (`reused`), его держатели (`holders`), остался ли он жить (`kept`) и почему нет (`reason`) ([правило](a-receipt-says-whether-the-agent-was-live.md)).

**Что изменила версия 8.** Общая квитанция `provider` получила необязательное поле `reason` (`whole` или `by_changes`) — почему цепочка умолчаний стоит в этом порядке. У этой команды поле допускается схемой, но не пишется: объём упорядочивает цепочку только у `push` и `pull` файловой базы ([правило](../use-cases/the-agent-leads-a-default-chain.md)).

**Что изменила версия 7.** Квитанция `provider` получила необязательное поле
`endpoint` — точку входа сессии агента, через которую шло исполнение: `mode`
(`managed`, `attached` или `gate`) и `address` — `host:port` подключения без учётных
данных. Поле есть, только когда команда открыла сессию агента; у процесса платформы,
у превью и у отказа до подключения его нет ([правило](a-session-receipt-names-its-endpoint.md)). Прежде квитанция
точку входа не называла.

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
