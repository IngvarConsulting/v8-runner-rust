---
id: CTR.WIRE.INFOBASE-DUMP-DATA
version: 7
artifact: docs/schemas/command-data/infobase-dump.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it
---

# `data` команды `infobase dump`

Снимок базы целиком (`.dt`) отчитывается той же логикой выбора провайдера, что и выгрузка
пакета, но своим предметом: `subject.kind` здесь — вся база, а не конфигурация внутри неё.
Формы разведены, потому что состав полей у них расходится и будет расходиться дальше.

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
