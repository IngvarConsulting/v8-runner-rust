---
id: INV.USE-CASES.AN-IBCMD-FULL-DUMP-LANDS-OVER-THE-DIRECTORY-THROUGH-A-STAGE
check: [src/use_cases/dump_config.rs::an_ibcmd_full_dump_lands_over_a_non_empty_directory_through_a_stage]
---

# Полная выгрузка `ibcmd` ложится поверх каталога через промежуточный

`ibcmd config export` без `--sync` в непустой каталог отказывает
([замер](../../../references/1c/confirmed-runtime-measurements.md), раздел о версии формата
файла версий). Поэтому полную выгрузку поверх каталога набора — без файла версий, с чужим
файлом или с чужой версией формата — `ibcmd` делает в пустой промежуточный каталог раннера, а
раннер переносит результат поверх каталога набора так же, как Конфигуратор пишет его на место:
файлы выгрузки переписаны, лишнее и посторонние файлы, `.git` в их числе, остаются
(`INV.CLI.PULL-LAYS-THE-DUMP-OVER-THE-DIRECTORY`). Решение владельца от 08.10.2026.
