---
id: INV.CLI.PULL-LAYS-THE-DUMP-OVER-THE-DIRECTORY
check:
  - tests/cli_dump.rs::a_pull_lays_the_dump_over_the_directory_and_force_brings_it_to_the_base
  - tests/cli_dump.rs::an_incremental_pull_refuses_over_work_version_control_cannot_give_back
  - tests/cli_dump.rs::an_edt_project_is_replaced_only_by_the_same_confirmation_rules
  - tests/cli_pull_memory.rs::a_source_edit_keeps_the_pull_incremental
  - tests/cli_pull_memory.rs::a_full_dump_over_the_directory_asks_the_replacement_guard
---

# Выгрузка ложится поверх каталога пообъектно

`pull` кладёт выгруженное поверх каталога так, как платформа отдаёт изменившееся; чего в
базе нет, остаётся на месте. Слияния версий раннер не делает: историю, трёхстороннее
слияние и разметку конфликтов держит система контроля версий. Правка исходников в каталоге
выгрузку полной не делает: режим выбирает файл версий, а что станет с правкой, решает
слияние в системе контроля версий.

`pull --force` приводит каталог ровно к базе, и лишнее исчезает. Для формата EDT слияния
нет ни в одном режиме: выгрузка либо заменяет проект по тем же правилам подтверждения,
либо отказывает.

Сторож безвозвратного срабатывает и на пообъектную перезапись, до запуска платформы: какие
файлы перепишет выгрузка по изменившемуся, заранее неизвестно, поэтому незафиксированное в
каталоге набора останавливает и её, и выборку объектов
(`INV.USE-CASES.REPLACING-A-USER-DIRECTORY-ASKS-FIRST`). Какой режим случился на самом деле,
ответ называет по прогнозу платформы — это отдельное обязательство
`INV.USE-CASES.THE-DUMP-MODE-IS-FORECAST-IN-THE-SAME-COMMAND`.
