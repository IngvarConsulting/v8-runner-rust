---
id: CTR.WIRE.INFOBASE-CREATE-DATA
version: 4
artifact: docs/schemas/command-data/infobase-create.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it
  - tests/cli_init.rs::init_dry_run_plans_the_infobase_without_creating_it
  - tests/cli_infobase_copy.rs::debugging_on_a_copy_of_the_base_leaves_the_neighbour_untouched
  - tests/cli_infobase_copy.rs::a_preview_names_the_snapshot_and_a_wrong_source_is_refused
---

# `data` команды `infobase create`

Команда доводит окружение до состояния, в котором можно собирать: создаёт базу и, для
формата EDT, рабочее пространство. Форма перечисляет шаги со статусом каждого, поэтому
пропущенный шаг виден так же явно, как сделанный, и не притворяется успехом.

Под превью шаг, который был бы выполнен, отвечает `status: planned`, а `provider_dispatched` —
`false`: план построен, платформа найдена, но ничего не создано. Что значит признак, говорит
[общее правило](provider-dispatched-says-whether-an-executor-got-work.md).

У `infobase create --from` форма называет источник копии — поле `source`: имя базы-источника
в местном слое (`infobase`) и абсолютный путь образа DT под `workPath` (`snapshot`); под
превью — путь, куда образ лёг бы. Без `--from` и пока база-источник не найдена, поля нет.

**Что изменила версия 4.** Появилось необязательное поле `source` — источник копии у
`infobase create --from` (решение владельца от 05.10.2026, #330). Ответ без `--from` прежний.

**Что изменила версия 3.** Схема квитанции `provider` допускает необязательное поле
`endpoint` — точку входа сессии агента ([правило](a-session-receipt-names-its-endpoint.md)): тип квитанции
общий у всех команд. `infobase create` через сессию агента не идёт, и в её ответе поля нет;
значения на проводе прежние.

## Пример

```json
{
  "provider": {"selected": "designer", "origin": {"kind": "default"}},
  "ok": true,
  "provider_dispatched": false,
  "duration_ms": 0,
  "steps": [
    {
      "target": "infobase",
      "action": "create",
      "status": "planned",
      "message": "would snapshot the infobase 'upstream' (file infobase '/work/erp/build/ib') to '/work/erp-wt/work/copies/upstream.dt' via /opt/1cv8/bin/1cv8 /DumpIB — the source must be free, the runner ends no sessions — and create file infobase '/work/erp-wt/build/ib' from it via /opt/1cv8/bin/1cv8 /RestoreIB",
      "duration_ms": 0
    },
    {
      "target": "edt_workspace",
      "action": "import",
      "status": "skipped",
      "message": "EDT workspace initialization is not applicable for format=DESIGNER",
      "duration_ms": 0
    }
  ],
  "source": {
    "infobase": "upstream",
    "snapshot": "/work/erp-wt/work/copies/upstream.dt"
  }
}
```
