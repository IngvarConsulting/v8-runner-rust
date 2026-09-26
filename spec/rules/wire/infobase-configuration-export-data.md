---
id: CTR.WIRE.INFOBASE-CONFIGURATION-EXPORT-DATA
version: 5
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

**Что изменила версия 5.** Значение `export_or_publication` ушло из общего набора фаз; эта
форма его не давала. Каждая остановка на безопасной точке теперь пишет прерывание: статус
`cancelled` и запись с фазой `command_boundary`, а не `failed` без записи. Процесс, которому
отмена не дала запуститься, тоже называется `command_boundary`, а не `provider_command`:
работы он не получил. Имена шагов в `steps[]` прежние.

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
      {"provider": "designer", "reason": "file infobase is not ready: 'build/ib/1Cv8.1CD' is missing or is not a file"},
      {"provider": "ibcmd", "reason": "file infobase is not ready: 'build/ib/1Cv8.1CD' is missing or is not a file"}
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
