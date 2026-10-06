---
id: CTR.WIRE.DOWNLOAD-DATA
version: 7
artifact: docs/schemas/command-data/download.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it
---

# `data` команды `download`

Выгрузка пакета конфигурации из базы отчитывается не только результатом, но и выбором
исполнителя: квитанция `provider` называет выбранного, откуда взялся выбор (умолчание
матрицы или ключ `providers.*` с именем файла) и каждого пропущенного до него с причиной.
`selected: null` — выбор состоялся и никто не подошёл; отсутствие квитанции — выбор не
начинался, команда отказала раньше.

`target_state` говорит о состоянии базы после операции: выгрузка не меняет базу, и форма
это утверждает явно.

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
