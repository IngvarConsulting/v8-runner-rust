---
id: CTR.WIRE.INFOBASE-CONFIGURATION-EXPORT-DATA
version: 6
artifact: docs/schemas/command-data/download.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it
---

# `data` команды `infobase configuration export`

Выгрузка пакета конфигурации из базы отчитывается не только результатом, но и выбором
исполнителя: квитанция `provider` называет выбранного, откуда взялся выбор (умолчание
матрицы или ключ `providers.*` с именем файла) и каждого пропущенного до него с причиной.
`selected: null` — выбор состоялся и никто не подошёл; отсутствие квитанции — выбор не
начинался, команда отказала раньше.

`target_state` говорит о состоянии базы после операции: выгрузка не меняет базу, и форма
это утверждает явно.

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
  "state": "working",
  "subject": {
    "kind": "main"
  },
  "provider": {
    "selected": null,
    "origin": {"kind": "default"},
    "skipped": [
      {"provider": "designer", "reason": "file infobase is not ready: '/home/dev/project/build/ib/1Cv8.1CD' is missing or is not a file"},
      {"provider": "ibcmd", "reason": "file infobase is not ready: '/home/dev/project/build/ib/1Cv8.1CD' is missing or is not a file"}
    ]
  },
  "artifact_kind": "cf",
  "output": "/home/dev/project/build/main.cf",
  "published": false,
  "target_state": "unchanged",
  "execution": {
    "status": "failed",
    "errors": [
      {
        "code": "environment_unavailable",
        "message": "environment unavailable: designer: file infobase is not ready: '/home/dev/project/build/ib/1Cv8.1CD' is missing or is not a file; ibcmd: file infobase is not ready: '/home/dev/project/build/ib/1Cv8.1CD' is missing or is not a file"
      }
    ]
  }
}
```
