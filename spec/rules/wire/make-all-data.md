---
id: CTR.WIRE.MAKE-ALL-DATA
version: 1
artifact: docs/schemas/command-data/make-all.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/cli_make_download_all.rs::make_without_a_set_builds_every_set_into_the_directory
  - tests/cli_make_download_all.rs::make_without_a_set_preview_builds_nothing
---

# `data` команды `make` без набора

`make` без набора отвечает своей формой рядом с формой `make <SET>` (`CTR.WIRE.MAKE-DATA`):
команда в конверте та же, `make`, а форму выбирает отсутствие набора и `--extension`.

`output_path` — каталог `--output`, разрешённый от текущего каталога. `sets` — сборка каждого набора формой `make <SET>` в
порядке обхода; после первого отказа обход останавливается, и последняя запись — отказавший
набор. `provider_dispatched` — получил ли исполнитель работу хотя бы у одного набора; у превью
`false`.

## Пример

```json
{
  "ok": true,
  "provider_dispatched": true,
  "output_path": "/home/dev/project/dist",
  "sets": [
    {
      "provider": {"selected": "designer", "origin": {"kind": "default"}},
      "ok": true,
      "provider_dispatched": true,
      "mode": "configuration_cf",
      "source_set": "main",
      "output_path": "/home/dev/project/dist/main.cf",
      "duration_ms": 4100,
      "execution": {
        "status": "succeeded",
        "payload": {
          "artifact_type": "configuration_cf",
          "output_path": "/home/dev/project/dist/main.cf",
          "file_names": ["main.cf"],
          "published": true
        }
      }
    },
    {
      "provider": {"selected": "designer", "origin": {"kind": "default"}},
      "ok": true,
      "provider_dispatched": true,
      "mode": "extension_cfe",
      "source_set": "Sales",
      "extension": "Sales",
      "output_path": "/home/dev/project/dist/Sales.cfe",
      "duration_ms": 2300,
      "execution": {
        "status": "succeeded",
        "payload": {
          "artifact_type": "extension_cfe",
          "output_path": "/home/dev/project/dist/Sales.cfe",
          "file_names": ["Sales.cfe"],
          "published": true
        }
      }
    }
  ],
  "duration_ms": 6500
}
```
