---
id: INV.USE-CASES.AN-IBCMD-FULL-DUMP-LANDS-OVER-THE-DIRECTORY-THROUGH-A-STAGE
check:
  - src/use_cases/dump_config.rs::an_ibcmd_full_dump_lands_over_a_non_empty_directory_through_a_stage
  - src/use_cases/dump_config.rs::a_failed_ibcmd_stage_dump_leaves_the_directory_untouched
  - src/use_cases/dump_config.rs::a_broken_overlay_keeps_the_runner_copy_as_it_was
  - src/use_cases/dump_config.rs::an_ibcmd_refusal_to_export_by_changes_falls_back_to_a_full_export
---

# Полная выгрузка `ibcmd` ложится поверх каталога через промежуточный

`ibcmd config export` в непустой каталог отказывает
([замер](../../../references/1c/confirmed-runtime-measurements.md), раздел о версии формата
файла версий). Поэтому полную выгрузку поверх каталога набора — без годной копии описи, с
чужой версией формата, по прогнозу «полная» или когда `config export --base` отказывает
«Требуется экспортировать конфигурацию полностью» (замер #646, удаление объекта) — `ibcmd` делает
в пустой промежуточный каталог раннера, а раннер переносит результат поверх каталога набора так
же, как у Конфигуратора: файлы выгрузки переписаны, лишнее и посторонние файлы, `.git` в их
числе, остаются (`INV.CLI.PULL-LAYS-THE-DUMP-OVER-THE-DIRECTORY`). Опись в каталог не
переносится, а становится копией раннера; оборванный перенос копию не меняет
(`INV.USE-CASES.THE-RUNNER-COPY-TAKES-THE-PLATFORM-FILE-AFTER-SUCCESS`). Другой отказ
`--base` с другим кодом — отказ выгрузки; решает код `-1`, а не текст.
