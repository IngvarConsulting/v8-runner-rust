---
id: CTR.WIRE.MAKE-DATA
version: 3
artifact: docs/schemas/command-data/make.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it
---

# `data` команды `make`

Сборка поставляемого артефакта отчитывается его видом, путём и — во вложенном
`execution.payload` — именами файлов, которые легли на диск. Имена нужны отдельно от пути:
у поставки из нескольких файлов путь один, а файлов много.

`published: false` при успешном исполнении означает превью: артефакт спланирован, но не
выложен.

**Что изменила версия 3.** Фаза прерывания `execution.interruptions[].phase` стала закрытым
набором значений в `snake_case`, общим для всех форм с итогом исполнения; набор перечисляет
`$defs/ExecutionInterruptionPhase` схемы. `publish` стал `publication`, `export_or_publish` —
`export_or_publication`. Последнее значение получает любой отказ, пришедший, когда прерывание
уже запрошено, и остановку на безопасной точке перед экспортом тоже: какую работу прервали,
ответ не различает ([#308](https://github.com/IngvarConsulting/v8-runner-rust/issues/308)).

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
