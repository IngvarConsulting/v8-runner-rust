---
id: CTR.WIRE.MAKE-DATA
version: 4
artifact: docs/schemas/command-data/make.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it
  - src/use_cases/artifacts.rs::a_designer_export_cancelled_after_its_start_is_a_cut_provider_command
  - src/use_cases/artifacts.rs::an_unrelated_failure_while_an_interruption_is_pending_stays_a_failure
---

# `data` команды `make`

Сборка поставляемого артефакта отчитывается его видом, путём и — во вложенном
`execution.payload` — именами файлов, которые легли на диск. Имена нужны отдельно от пути:
у поставки из нескольких файлов путь один, а файлов много.

`published: false` при успешном исполнении означает превью: артефакт спланирован, но не
выложен.

**Что изменила версия 4.** Значение `export_or_publication` ушло из набора фаз
`execution.interruptions[].phase`: прерывание называет, что прервано. Остановка на безопасной
точке — перед экспортом или перед публикацией — даёт `command_boundary`, выгрузка, снятая после
запуска Конфигуратора, — `provider_command`. Отказ, пришедший, когда прерывание уже запрошено,
остаётся отказом: `status: failed` и ошибка `designer_export_failed`, без записи о прерывании.
Набор фаз общий для всех форм с итогом исполнения, и версию они сменили вместе.

## Пример

```json
{
  "provider": {"selected": "designer", "origin": {"kind": "default"}},
  "ok": true,
  "provider_dispatched": false,
  "mode": "configuration_cf",
  "source_set": "main",
  "output_path": "build/main.cf",
  "duration_ms": 0,
  "message": "would build ConfigurationCf into 'build/main.cf' via /opt/1cv8/bin/1cv8; nothing published",
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
