---
id: CTR.WIRE.INFOBASE-DUMP-DATA
version: 5
artifact: docs/schemas/command-data/infobase-dump.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it
---

# `data` команды `infobase dump`

Снимок базы целиком (`.dt`) отчитывается той же логикой выбора провайдера, что и выгрузка
пакета, но своим предметом: `subject.kind` здесь — вся база, а не конфигурация внутри неё.
Формы разведены, потому что состав полей у них расходится и будет расходиться дальше.

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
  "subject": {
    "kind": "infobase"
  },
  "provider": {
    "selected": null,
    "origin": {"kind": "default"},
    "skipped": [{"provider": "designer", "reason": "file infobase is not ready: 'build/ib/1Cv8.1CD' is missing or is not a file"}]
  },
  "artifact_kind": "dt",
  "output": "build/main.dt",
  "published": false,
  "target_state": "unchanged",
  "execution": {
    "status": "failed",
    "errors": [
      {
        "code": "environment_unavailable",
        "message": "environment unavailable: no provider is ready"
      }
    ]
  }
}
```
