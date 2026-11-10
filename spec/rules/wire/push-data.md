---
id: CTR.WIRE.PUSH-DATA
version: 6
artifact: docs/schemas/command-data/push.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/contract_command_data.rs::every_previewable_command_answers_in_the_form_declared_for_it
  - tests/mcp_stdio.rs::mcp_stdio_tools_answer_in_the_forms_of_their_commands
  - tests/cli_apply.rs::a_push_without_apply_loads_and_apply_applies_it
  - tests/contract_receipt.rs::a_file_base_picks_the_executor_by_the_size_of_the_operation
  - tests/cli_status.rs::status_and_push_name_how_the_changes_were_found
  - tests/cli_apply_sessions.rs::force_on_a_cluster_ends_the_sessions_and_names_them
---

# `data` команды `push`

Сборка идёт по наборам исходников, и форма отчитывается по каждому: какой набор, каким
режимом загружен и почему. Режим выбирают правила частичной загрузки, а не вызывающий,
поэтому `mode` в ответе — решение раннера, и в нём же его причина.

Инструмент MCP `build_project` отвечает этой же формой: поверхность MCP не повторяет CLI,
но предмет команды у них один.

**Что изменила версия 6.** У квитанции исполнителя в `endpoint` управляемого агента новое поле `live`: взяла ли команда уже живого агента (`reused`), его держатели (`holders`), остался ли он жить (`kept`) и почему нет (`reason`) ([правило](a-receipt-says-whether-the-agent-was-live.md)). Новое необязательное поле `detection`: по имени каждого набора, изменения которого команда разбирала, — каким путём они найдены, той же формой, что у `status` (`path` — `git`, `git+fsmonitor` или `scan`, у `scan` — `reason`). Поля нет, когда команда изменений не разбирала: полная отправка, отказ до разбора ([правило](../use-cases/changes-are-detected-on-demand.md)). В той же версии — необязательное поле `sessions`: исход `--sessions force` — завершённые сеансы `terminated` из списка до завершения, блокировка начала сеансов `lock` и `message_delivered`; без `force` поля нет ([правило](../cli/apply-sessions-governs-foreign-sessions.md)).

**Что изменила версия 5.** Квитанция `provider` получила необязательное поле `reason` — почему
цепочка умолчаний стоит в этом порядке: `whole` — каждый набор команды идёт целиком, и первым
стоит `ibcmd`; `by_changes` — хоть один набор идёт по изменившемуся или выборкой, и первым
стоит агент ([правило](../use-cases/the-agent-leads-a-default-chain.md)). Поле есть только у
умолчания файловой базы, которое раннер упорядочил по объёму; у ключа `providers.*`, у кластера,
автономного сервера и подключённого агента его нет. Прежде объём выбор не менял.

**Что изменила версия 4.** У шага новое обязательное поле `applied`: дошёл ли он до
конфигурации базы данных — загруженное применено или применено непринятое прежней загрузки.
`false` у пропуска, который ничего не применил, у превью, `push --no-apply` и у отказа
([правило](../cli/apply-is-a-separate-step.md)). Прежде применение было неотделимо от загрузки, и
поле не требовалось.

**Что изменила версия 3.** Квитанция `provider` получила необязательное поле
`endpoint` — точку входа сессии агента, через которую шло исполнение: `mode`
(`managed`, `attached` или `gate`) и `address` — `host:port` подключения без учётных
данных. Поле есть, только когда команда открыла сессию агента; у процесса платформы,
у превью и у отказа до подключения его нет ([правило](a-session-receipt-names-its-endpoint.md)). Прежде квитанция
точку входа не называла.

## Пример

```json
{
  "provider": {"selected": "designer", "origin": {"kind": "default"}},
  "ok": true,
  "provider_dispatched": false,
  "duration_ms": 1,
  "steps": [
    {
      "source_set": "main",
      "mode": "full",
      "ok": true,
      "applied": false,
      "message": "full load selected by partial-load rules; planned, Designer not dispatched",
      "duration_ms": 0
    }
  ]
}
```
