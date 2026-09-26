---
id: CTR.WIRE.EXTENSIONS-INVENTORY-DATA
version: 4
artifact: docs/schemas/command-data/extensions-inventory.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it
  - tests/cli_extensions.rs::extensions_info_that_fails_after_the_platform_ran_answers_in_its_form
---

# `data` чтения состава расширений

Так отвечают `extensions list` и `extensions info`: составом установленных расширений в
том порядке, в каком его назвала платформа. Порядок не документирован и не является
порядком создания — опираться на него нельзя, и форма этого не обещает.

Чтение состава — действие, а не взгляд: платформа стартует, сессия открывается, в журнале
остаётся след. Поэтому у него тоже есть превью, и в нём `extensions` пуст: ничего не
спрашивали.

У записи есть `name_prefix`: строка означает префикс из применённой конфигурации БД
(пустая строка — известный пустой префикс), `null` — поставщик не может подтвердить
это свойство. `ibcmd` читает его из сохранённого состояния БД и сверяет запись
с инвентаризацией; агент пока сообщает `null`. Подмена рабочей конфигурацией
после `upload` до `apply` недопустима.

**Что изменила версия 4.** Отказ после того, как исполнитель получил работу, отвечает
этой формой: `ok: false`, `extensions` пуст — состав неизвестен, а не пуст, — и
`provider_dispatched: true`. Прежде любой отказ чтения отвечал общей формой отказа.

Предмет чтения назван полем `requested` — `{"kind": "all"}` или
`{"kind": "named", "name": …}` — и в превью, и в ответе: вызывающий сверяет ответ со
своим запросом по нему, а не по формулировке `plan`. `plan` остаётся для человека.

## Пример

```json
{
  "provider": {"selected": "ibcmd", "origin": {"kind": "default"}},
  "ok": true,
  "provider_dispatched": false,
  "requested": {"kind": "all"},
  "plan": "would read every installed extension of file infobase 'build/ib' with no configured infobase user via /opt/1cv8/bin/ibcmd",
  "extensions": [],
  "duration_ms": 0
}
```
