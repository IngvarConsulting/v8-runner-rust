---
id: CTR.WIRE.DIFF-DATA
version: 1
artifact: docs/schemas/command-data/diff.schema.json
check:
  - src/command_data.rs::generated_command_data_schemas_are_current
  - tests/cli_diff.rs::diff_lists_metadata_versions_without_dumping
  - tests/cli_diff.rs::designer_lists_named_objects_and_refuses_unrecognized_entries
  - tests/cli_diff.rs::full_dump_and_empty_list_are_distinct_for_both_providers
  - src/platform/dump_forecast.rs::only_the_utf8_signature_is_a_valid_empty_designer_diff
  - tests/cli_diff.rs::against_keeps_opaque_report_and_uses_typed_designer_arguments
  - tests/cli_diff.rs::designer_output_must_exist_and_partial_reports_are_discarded
  - tests/cli_diff.rs::platform_failure_includes_stdout_without_the_infobase_password
  - tests/cli_diff.rs::against_rejects_an_explicit_ibcmd_override_without_dispatch
  - tests/cli_diff.rs::owner_boundary_warns_for_status_and_preserves_marker_for_both_forms
  - tests/cli_diff.rs::version_file_is_resolved_without_rewriting_the_sources
  - src/platform/designer.rs::full_comparison_passes_windows_paths_without_verbatim_prefix
---

# `data` команды `diff`

Обычный `diff` возвращает в `changes` имена объектов основной конфигурации с изменившимися
версиями. Каждый элемент несёт `object` и `kind` (`added` или `modified`). Отсчёт задаёт
`ConfigDumpInfo.xml` основного набора. Для проекта EDT берётся снимок Конфигуратора.
Подменённый файл уступает копии той же пары, если она есть, по той же проверке, что у `pull`.
При отсутствии своей копии отсчёт задаёт файл каталога. Исходники и память не обновляются.
Без файла команда отказывает до запуска исполнителя.

`full_dump: true` и пустой `changes` означают, что платформа требует полную выгрузку и не
перечислила объекты. `full_dump: false` и пустой `changes` у успешного обычного `diff` означают,
что платформа не нашла изменений версий. Список описывает версии, а не содержимое: после
`push` в нём все загруженные объекты, пока выгрузка не обновит файл версий: `pull` при
неизменном поколении не выгружает, файл обновляет `pull --force`. Неизвестный ответ платформы даёт `invalid_output`.
Designer обозначает пустой список одной сигнатурой UTF-8 BOM. `ibcmd` отдаёт список файлом
`--out` в UTF-8: stdout на Windows идёт в кодовой странице OEM и теряет символы вне неё.
[Замер на платформе 8.3.27.2074](../../../references/1c/confirmed-runtime-measurements.md).

`diff --against db|vendor:<имя>|<файл>` при успехе возвращает абсолютный путь `report` к
полному TXT-отчёту Конфигуратора в UTF-16LE с BOM. Раннер сохраняет байты отчёта. В этой форме `changes`
пуст и `full_dump` равен `false`. Путь появляется только у успешного непустого файла.

`provider` объясняет выбор. `provider_dispatched` берётся из отметки работы исполнителя.
Обычный `diff` проходит границу владельца как команда записи, `diff --against` — как
команда чтения, по `INV.USE-CASES.A-DEVELOPMENT-BASE-IS-HELD-BY-ONE-WORKING-COPY`.

## Пример

```json
{
  "provider": {"selected": "ibcmd", "origin": {"kind": "default"}},
  "provider_dispatched": true,
  "changes": [{"object": "Catalog.Products", "kind": "modified"}],
  "full_dump": false,
  "duration_ms": 120
}
```

```json
{
  "provider": {"selected": "designer", "origin": {"kind": "default"}},
  "provider_dispatched": true,
  "changes": [],
  "full_dump": false,
  "report": "/work/reports/diff/.tmp7p8CvR/report.txt",
  "duration_ms": 120
}
```
