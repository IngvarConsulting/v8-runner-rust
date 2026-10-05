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

**Что изменила версия 5.** Значение `export_or_publication` ушло из общего набора фаз; эта
форма его не давала. Каждая остановка на безопасной точке теперь пишет прерывание: статус
`cancelled` и запись с фазой `command_boundary`, а не `failed` без записи. Процесс, которому
отмена не дала запуститься, тоже называется `command_boundary`, а не `provider_command`:
работы он не получил. Имена шагов в `steps[]` прежние.

**Что изменила версия 6.** Причина пропуска исполнителя одна — почему он не взят:
приставка о реализованности адаптера (`Designer CF/CFE adapter is implemented …; `) ушла из
`provider.skipped[].reason`, а с ней из `execution.errors[].message`, `steps[].message` и
`error.message`. Исполнитель вне матрицы, названный ключом `providers.*`, до выбора не
доходит: команда отказывает при загрузке настроек родом `invalid_argument`, как `push`, и
квитанции в ответе нет. Путь в причине — канонизованный абсолютный.

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
