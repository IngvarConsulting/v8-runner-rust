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

Отказ исполнителя, так и не получившего работу, базу не трогал:
`target_state` остаётся `unchanged`, и предупреждения о неудавшемся откате в ответе нет.

**Что изменила версия 6.** Значения на проводе прежние; изменились имена типов в схеме и
код шага у недостающей утилиты. Типы, общие для выгрузки, снимка и подъёма базы, названы без
слова «export», которое врало подъёму: `$defs.ExportTargetState` стал
`$defs.InfobaseTargetState`, `$defs.InfobaseExportMode` — `$defs.InfobaseTransferMode`,
`$defs.InfobaseExportArtifactKind` — `$defs.TransferArtifactKind`; ссылки `$ref` и описания
этих типов следуют за ними. Код шага в `execution.errors[]` выводится из того же рода
отказа, что и род конверта, а не из своего отображения ошибок. Утилита, которой нет, или не
той версии, или с нечитаемой версией, пишет там `environment_unavailable`, как и род
`environment` конверта; прежнее отображение давало `platform_failure` всякому такому отказу,
дошедшему до шага после выбора исполнителя. Выбор, не нашедший готового исполнителя, и прежде
писал `environment_unavailable`. Прочие коды шага прежние.

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
