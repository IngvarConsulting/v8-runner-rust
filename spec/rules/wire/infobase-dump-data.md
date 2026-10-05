---
id: CTR.WIRE.INFOBASE-DUMP-DATA
version: 6
artifact: docs/schemas/command-data/infobase-dump.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it
---

# `data` команды `infobase dump`

Снимок базы целиком (`.dt`) отчитывается той же логикой выбора провайдера, что и выгрузка
пакета, но своим предметом: `subject.kind` здесь — вся база, а не конфигурация внутри неё.
Формы разведены, потому что состав полей у них расходится и будет расходиться дальше.

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
  "provider": {
    "selected": null,
    "origin": {"kind": "default"},
    "skipped": [{"provider": "designer", "reason": "file infobase is not ready: '/home/dev/project/build/ib/1Cv8.1CD' is missing or is not a file"}]
  },
  "artifact_kind": "dt",
  "output": "/home/dev/project/build/main.dt",
  "published": false,
  "target_state": "unchanged",
  "execution": {
    "status": "failed",
    "errors": [
      {
        "code": "environment_unavailable",
        "message": "environment unavailable: designer: file infobase is not ready: '/home/dev/project/build/ib/1Cv8.1CD' is missing or is not a file"
      }
    ]
  }
}
```
