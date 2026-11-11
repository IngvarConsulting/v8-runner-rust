---
id: CTR.WIRE.DOWNLOAD-ALL-DATA
version: 3
artifact: docs/schemas/command-data/download-all.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/cli_make_download_all.rs::download_without_a_set_downloads_the_installed_packages
  - tests/cli_make_download_all.rs::download_without_a_set_preview_reads_nothing
---

# `data` команды `download` без набора

`download` без набора отвечает своей формой рядом с формой `download <SET>`
(`CTR.WIRE.DOWNLOAD-DATA`): команда в конверте та же, `download`, а форму выбирает отсутствие
набора и `--extension`.

`provider` — квитанция исполнителя, который читал состав базы и выгружал пакеты. `output` —
каталог `--output`, разрешённый от `basePath`. `sets` — выгрузка каждого пакета формой `download <SET>` в порядке обхода;
после первого отказа обход останавливается, и последняя запись — отказавший набор. У превью в
`sets` только наборы конфигурации. `not_installed` — наборы расширений проекта, которых в базе
нет: их не выгружали. `if_installed` бывает только у превью — наборы расширений проекта,
которые прогон выгрузит, если их расширение в базе есть. Пустые `not_installed` и
`if_installed` не пишутся.

**Что изменила версия 3.** У квитанции исполнителя в `endpoint` управляемого агента новое поле `live`: взяла ли команда уже живого агента (`reused`), его держатели (`holders`), остался ли он жить (`kept`) и почему нет (`reason`) ([правило](a-receipt-says-whether-the-agent-was-live.md)).

**Что изменила версия 2.** Общая квитанция `provider` получила необязательное поле `reason` (`whole` или `by_changes`) — почему цепочка умолчаний стоит в этом порядке. У этой команды поле допускается схемой, но не пишется: объём упорядочивает цепочку только у `push` и `pull` файловой базы ([правило](../use-cases/the-agent-leads-a-default-chain.md)).

## Пример

```json
{
  "provider": {"selected": "designer", "origin": {"kind": "default"}},
  "ok": true,
  "provider_dispatched": true,
  "output": "/home/dev/project/dist",
  "not_installed": ["Old"],
  "sets": [
    {
      "mode": "apply",
      "state": "working",
      "subject": {"kind": "main"},
      "provider": {"selected": "designer", "origin": {"kind": "default"}},
      "artifact_kind": "cf",
      "output": "/home/dev/project/dist/main.cf",
      "published": true,
      "target_state": "unchanged",
      "execution": {"status": "succeeded"}
    },
    {
      "mode": "apply",
      "state": "working",
      "subject": {"kind": "extension", "name": "Sales"},
      "provider": {"selected": "designer", "origin": {"kind": "default"}},
      "artifact_kind": "cfe",
      "output": "/home/dev/project/dist/Sales.cfe",
      "published": true,
      "target_state": "unchanged",
      "execution": {"status": "succeeded"}
    }
  ],
  "duration_ms": 9100
}
```
