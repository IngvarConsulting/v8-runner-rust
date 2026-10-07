---
id: CTR.WIRE.INFOBASE-CREATE-DATA
version: 3
artifact: docs/schemas/command-data/infobase-create.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it
  - tests/cli_init.rs::init_dry_run_plans_the_infobase_without_creating_it
---

# `data` команды `infobase create`

Команда доводит окружение до состояния, в котором можно собирать: создаёт базу и, для
формата EDT, рабочее пространство. Форма перечисляет шаги со статусом каждого, поэтому
пропущенный шаг виден так же явно, как сделанный, и не притворяется успехом.

Под превью шаг, который был бы выполнен, отвечает `status: planned`, а `provider_dispatched` —
`false`: план построен, платформа найдена, но ничего не создано. Что значит признак, говорит
[общее правило](provider-dispatched-says-whether-an-executor-got-work.md).

**Что изменила версия 3.** Схема квитанции `provider` допускает необязательное поле
`endpoint` — точку входа сессии агента ([правило](a-session-receipt-names-its-endpoint.md)): тип квитанции
общий у всех команд. `infobase create` через сессию агента не идёт, и в её ответе поля нет;
значения на проводе прежние.

## Пример

```json
{
  "provider": {"selected": "ibcmd", "origin": {"kind": "default"}},
  "ok": true,
  "provider_dispatched": false,
  "duration_ms": 0,
  "steps": [
    {
      "target": "infobase",
      "action": "create",
      "status": "planned",
      "message": "would create a file infobase at 'build/ib' with the main configuration of source-set 'main' via /opt/1cv8/bin/ibcmd",
      "duration_ms": 0
    },
    {
      "target": "edt_workspace",
      "action": "import",
      "status": "skipped",
      "message": "EDT workspace initialization is not applicable for format=DESIGNER",
      "duration_ms": 0
    }
  ]
}
```
