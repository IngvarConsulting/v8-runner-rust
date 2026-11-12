---
id: CTR.WIRE.EXTENSIONS-INVENTORY-DATA
version: 8
artifact: docs/schemas/command-data/extensions-inventory.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it
  - tests/cli_extensions.rs::extensions_info_that_fails_after_the_platform_ran_answers_in_its_form
  - tests/cli_extensions.rs::cluster_designer_lists_only_names_with_batch_argv
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
с инвентаризацией; агент сообщает `null` — и тогда, когда `extensions list` файловой базы
берёт уже живого агента
([правило](../use-cases/extensions-list-takes-the-live-agent.md)). Подмена рабочей конфигурацией
после `upload` до `apply` недопустима.

**Что изменила версия 8.** У квитанции исполнителя в `endpoint` управляемого агента новое поле `live`: взяла ли команда уже живого агента (`reused`), его держатели (`holders`), остался ли он жить (`kept`) и почему нет (`reason`) ([правило](a-receipt-says-whether-the-agent-was-live.md)).

**Что изменила версия 7.** Общая квитанция `provider` получила необязательное поле `reason` (`whole` или `by_changes`) — почему цепочка умолчаний стоит в этом порядке. У этой команды поле допускается схемой, но не пишется: объём упорядочивает цепочку только у `push` и `pull` файловой базы ([правило](../use-cases/the-agent-leads-a-default-chain.md)).

**Что изменила версия 6.** При чтении через пакетный Конфигуратор на кластере элемент
`extensions` содержит только `name`. Полные записи агента и `ibcmd` сохраняют прежние поля.

**Что изменила версия 5.** Квитанция `provider` получила необязательное поле
`endpoint` — точку входа сессии агента, через которую шло исполнение: `mode`
(`managed`, `attached` или `gate`) и `address` — `host:port` подключения без учётных
данных. Поле есть, только когда команда открыла сессию агента; у процесса платформы,
у превью и у отказа до подключения его нет ([правило](a-session-receipt-names-its-endpoint.md)). Прежде квитанция
точку входа не называла.

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
