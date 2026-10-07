---
id: CTR.WIRE.CONVERT-DATA
version: 2
artifact: docs/schemas/command-data/convert.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it
---

# `data` команды `convert`

Перевод между XML платформы, проектом EDT и пакетом отчитывается направлением, охватом и
списком того, что получилось на выходе. Направление в ответе обязательно: команда выбирает
его из `--to`, формата проекта и вида входа (`INV.CLI.CONVERT-DIRECTION-IS-SET-BY-TO`), и
вызывающий узнаёт выбор отсюда, а не из своих предположений.

У направления с пакетом ответ несёт квитанцию `provider` — кто исполнял и кого пропустили
(`INV.CLI.A-PACKAGE-DIRECTION-OF-CONVERT-HAS-AN-EXECUTOR-CHAIN`). Охват `PACKAGE` значит,
что на входе был файл пакета: у записи `outputs[]` тогда нет `source_set`. `workspace_path`
— рабочая область EDT, и его нет у направления, которое обходится без `1cedtcli`.

## Пример

```json
{
  "ok": true,
  "provider_dispatched": true,
  "direction": "DESIGNER_TO_PACKAGE",
  "scope": "SINGLE",
  "source_set": "Sales",
  "outputs": [
    {
      "source_set": "Sales",
      "source_path": "src/sales",
      "target_path": "build/convert/out/packages/Sales.cfe"
    }
  ],
  "provider": {
    "selected": "ibcmd",
    "origin": { "kind": "default" }
  },
  "duration_ms": 10420
}
```
