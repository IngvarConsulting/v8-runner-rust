---
id: CTR.WIRE.STATUS-DATA
version: 3
artifact: docs/schemas/command-data/status.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/cli_status.rs::status_without_deep_starts_no_platform
  - tests/cli_status.rs::status_deep_predicts_the_push_generation_check
  - tests/cli_status.rs::status_deep_without_a_platform_answers_null_with_a_reason
---

# `data` команды `status`

Форма отвечает о каждой базе ответа — выбранной или, у `--all`, о каждой объявленной в
местном слое — тем, что раннер о ней помнит: адрес без учётных данных, признак нового
владельца и по набору конфигурации и расширений — назначение (`purpose`, как ключ `type`
проекта: `CONFIGURATION` или `EXTENSION`), память о базе (`memory`), запись журнала
поколений (`recorded`) и число файлов каталога, изменившихся с последнего чтения
(`changed_files`; `null` — сравнить не с чем).

У `--deep` к набору добавляется `base`: поколение, которое ответил исполнитель `push`, и его
сверка с записью — `unchanged`, `moved_ahead`, `other_tool`, `no_record` или `no_answer`. К базе
добавляются `extensions` — состав расширений базы рядом с наборами проекта, у
расширения-инструмента клиентского MCP `tool: true`, — и, у файловой
базы, `holders` — копии из метки владельца. Без `--deep` этих полей нет. Что платформа не
ответила, форма называет `null` с причиной в `reason`, а не отказом команды.

**Что изменила версия 3.** У `base` новые поля: `unapplied` — есть ли в базе непринятое
(основная конфигурация, у набора расширения — само расширение, отличается от конфигурации
базы данных), `true`, `false` или `null`, и `unapplied_reason` — почему `null`
([правило](../cli/status-deep-names-the-unapplied.md)).

**Что изменила версия 2.** Копия в `holders.owners` — ровно `project`, `host`, `since` и
`this_copy`.

## Пример

```json
{
  "deep": true,
  "infobases": [
    {
      "name": "origin",
      "selected": true,
      "kind": "file",
      "address": "file:/srv/ib/demo",
      "new_owner_since": null,
      "source_sets": [
        {
          "name": "main",
          "purpose": "CONFIGURATION",
          "memory": "remembered",
          "recorded": {
            "token": "1111111111111111111111111111111111111111",
            "tool": "designer",
            "after": "build",
            "recorded_at": "2026-10-06T10:00:00+00:00"
          },
          "changed_files": 3,
          "base": {
            "tool": "designer",
            "token": "2222222222222222222222222222222222222222",
            "comparison": "moved_ahead",
            "unapplied": false
          }
        }
      ],
      "extensions": {
        "provider": {"selected": "ibcmd", "origin": {"kind": "default"}},
        "installed": [{"name": "Patch_007", "active": true, "source_set": null, "tool": false}],
        "missing_in_base": []
      },
      "holders": {
        "marker": "/srv/ib/.demo.v8-runner.owners.json",
        "owners": [
          {
            "project": "/home/dev/demo",
            "host": "dev-box",
            "since": "2026-10-01T09:00:00Z",
            "this_copy": true
          }
        ]
      }
    }
  ],
  "duration_ms": 4210
}
```
