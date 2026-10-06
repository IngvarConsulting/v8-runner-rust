---
id: CTR.WIRE.DOWNLOAD-ALL-DATA
version: 1
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
