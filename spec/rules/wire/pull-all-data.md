---
id: CTR.WIRE.PULL-ALL-DATA
version: 1
artifact: docs/schemas/command-data/pull-all.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/cli_pull_all.rs::an_extension_without_a_set_is_declared_and_pulled
  - tests/cli_pull_all.rs::a_pull_all_preview_reads_nothing_and_writes_nothing
---

# `data` команды `pull --all`

`pull --all` отвечает своей формой рядом с формой `pull <SET>` (`CTR.WIRE.PULL-DATA`):
команда в конверте та же, `pull`, а форму выбирает ключ `--all` вызова.

`sets` — выгрузка каждого набора формой `pull <SET>` в порядке обхода; после первого отказа
обход останавливается, и последняя запись — отказавший набор. У превью в `sets` только
набор основной конфигурации. `declared` — наборы, которые команда дописала в
`v8project.yaml`, полями `name`, `type`, `path`, как их называет ответ `init`.
`declared: null` — состав базы не читали: у превью, которое платформу не запускает, и у
отказа до чтения; пустой список — читали, и ничего не объявлено: объявлять было нечего или
обход отказал раньше. `not_installed` — наборы расширений проекта, которых в базе нет: их не
выгружали. `if_installed` бывает только у превью — наборы расширений проекта, которые прогон
выгрузит, если их расширение в базе есть. Пустые `not_installed` и `if_installed` не
пишутся.

## Пример

```json
{
  "provider": {"selected": "designer", "origin": {"kind": "default"}},
  "ok": true,
  "provider_dispatched": true,
  "declared": [{"name": "Sales", "type": "EXTENSION", "path": "src/ext/Sales"}],
  "not_installed": ["Old"],
  "sets": [
    {
      "provider": {"selected": "designer", "origin": {"kind": "default"}},
      "ok": true,
      "provider_dispatched": true,
      "up_to_date": false,
      "source_set": "main",
      "mode": "INCREMENTAL",
      "target_path": "src/cf",
      "duration_ms": 2900,
      "message": "dump completed successfully"
    },
    {
      "provider": {"selected": "designer", "origin": {"kind": "default"}},
      "ok": true,
      "provider_dispatched": true,
      "up_to_date": false,
      "source_set": "Sales",
      "extension": "Sales",
      "mode": "FULL",
      "target_path": "src/ext/Sales",
      "duration_ms": 3100,
      "message": "dump completed successfully"
    }
  ],
  "duration_ms": 8900
}
```
