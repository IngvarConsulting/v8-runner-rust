---
id: CTR.WIRE.MAKE-DATA
version: 6
artifact: docs/schemas/command-data/make.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it
  - src/use_cases/artifacts.rs::run_artifacts_honors_interruption_before_export_safe_point
  - src/use_cases/artifacts.rs::a_designer_export_cancelled_after_its_start_is_a_cut_provider_command
  - src/use_cases/artifacts.rs::an_unrelated_failure_while_an_interruption_is_pending_stays_a_failure
---

# `data` команды `make`

Сборка поставляемого артефакта отчитывается его видом, путём и — во вложенном
`execution.payload` — именами файлов, которые легли на диск. Имена нужны отдельно от пути:
у поставки из нескольких файлов путь один, а файлов много.

`published: false` при успешном исполнении означает превью: артефакт спланирован, но не
выложен.

Прерывание называет, что прервано. Остановка на безопасной точке — перед сборкой, перед
созданием временной базы, загрузкой, выгрузкой или публикацией — даёт запись
`command_boundary`, работа исполнителя, снятая после его запуска, — `provider_command`.
Рядом с записью стоят `status: cancelled` и ошибка `cancelled` в
`execution.errors[]` с тем же текстом. Отказ, пришедший, когда прерывание уже запрошено,
остаётся отказом: `status: failed` и ошибка `designer_export_failed` — так код называется у
любого исполнителя, — без записи о прерывании.

**Что изменила версия 6.** Квитанция `provider` получила необязательное поле
`endpoint` — точку входа сессии агента, через которую шло исполнение: `mode`
(`managed`, `attached` или `gate`) и `address` — `host:port` подключения без учётных
данных. Поле есть, только когда команда открыла сессию агента; у процесса платформы,
у превью и у отказа до подключения его нет ([правило](a-session-receipt-names-its-endpoint.md)). Прежде квитанция
точку входа не называла.

## Пример

```json
{
  "provider": {"selected": "ibcmd", "origin": {"kind": "default"}},
  "ok": true,
  "provider_dispatched": false,
  "mode": "configuration_cf",
  "source_set": "main",
  "output_path": "build/main.cf",
  "duration_ms": 0,
  "message": "would build ConfigurationCf into 'build/main.cf' via /opt/1cv8/bin/ibcmd in a throwaway infobase; nothing published",
  "execution": {
    "status": "succeeded",
    "diagnostics": [
      "would build ConfigurationCf into 'build/main.cf'; nothing published"
    ],
    "payload": {
      "artifact_type": "configuration_cf",
      "output_path": "build/main.cf",
      "file_names": [
        "main.cf"
      ],
      "published": false
    }
  }
}
```
