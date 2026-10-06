---
id: INV.CLI.PULL-LAYS-THE-DUMP-OVER-THE-DIRECTORY
check:
  - tests/cli_dump.rs::a_pull_lays_the_dump_over_the_directory_and_force_brings_it_to_the_base
  - tests/cli_dump.rs::an_edt_project_is_replaced_only_by_the_same_confirmation_rules
---

# Выгрузка ложится поверх каталога пообъектно

`pull` кладёт выгруженное поверх каталога так, как платформа отдаёт изменившееся; чего в
базе нет, остаётся на месте. Слияния версий раннер не делает: историю, трёхстороннее
слияние и разметку конфликтов держит система контроля версий.

`pull --force` приводит каталог ровно к базе, и лишнее исчезает. Для формата EDT слияния
нет ни в одном режиме: выгрузка либо заменяет проект по тем же правилам подтверждения,
либо отказывает.

Когда выгрузка останавливается над незафиксированным, держит
`INV.USE-CASES.REPLACING-A-USER-DIRECTORY-ASKS-FIRST`; какой режим случился на самом деле,
ответ называет по `INV.USE-CASES.THE-DUMP-MODE-IS-FORECAST-IN-THE-SAME-COMMAND`.
