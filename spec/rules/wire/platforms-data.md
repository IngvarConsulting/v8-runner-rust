---
id: CTR.WIRE.PLATFORMS-DATA
version: 1
artifact: docs/schemas/command-data/platforms.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/cli_platforms.rs::platforms_name_every_installation_and_the_choice_without_starting_anything
  - tests/cli_platforms.rs::without_a_version_the_sources_format_chooses_the_release
---

# `data` команды `platforms`

Установленные платформы и выбор раннера для проекта. Команда платформу не запускает и
ничего не пишет, кроме журнала действий структурного ответа.

`installations` — каждая установка, которую видит поиск, новейшие первыми, с составом
`components`; перечень не обрезается. `requirement` — `tools.platform` проекта вместе с
местным слоем. `source_format` — формат исходников основного набора и выпуск, который его
пишет. `selected` — утилита, которую выбрал бы тот же поиск, что у рабочих команд, по каждой
из `designer`, `thin_client`, `ibcmd`, или `null`, если её нет ни в одной подходящей
установке; `chosen_by` называет, почему выбрана эта версия: `version` — маска
`tools.platform.version`, `source_format` — выпуск формата исходников, `newest` —
версию ничто не задало. Расхождение формата исходников с выбранной платформой ответ
называет в `warnings`. В примере выпуск формата закреплён за всеми утилитами, и тонкого
клиента и `ibcmd` в нём нет: их не берут из 8.5.4.

## Пример

```json
{
  "installations": [
    {
      "version": "8.5.4.1878",
      "root": "/opt/1cv8/8.5.4.1878",
      "components": {"designer": true, "thin_client": true, "ibcmd": true, "web_extension": false}
    },
    {
      "version": "8.3.27.2074",
      "root": "/opt/1cv8/8.3.27.2074",
      "components": {"designer": true, "thin_client": false, "ibcmd": false, "web_extension": false}
    }
  ],
  "requirement": {"version": null, "path": null, "strict": false},
  "source_format": {"version": "2.20", "file": "/project/src/cf/Configuration.xml", "release": "8.3.27"},
  "selected": {
    "designer": {
      "path": "/opt/1cv8/8.3.27.2074/1cv8",
      "version": "8.3.27.2074",
      "source": "default-root",
      "installation_root": "/opt/1cv8/8.3.27.2074",
      "chosen_by": "source_format"
    },
    "thin_client": null,
    "ibcmd": null
  },
  "duration_ms": 12
}
```
