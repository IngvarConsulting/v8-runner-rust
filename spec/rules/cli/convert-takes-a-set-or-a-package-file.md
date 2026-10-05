---
id: INV.CLI.CONVERT-TAKES-A-SET-OR-A-PACKAGE-FILE
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/236
---

# Позиционный аргумент `convert` — набор или файл пакета

Аргумент с расширением `.cf` или `.cfe` считается файлом пакета, остальное — именем
набора исходников проекта. `--source-set <имя>` остаётся скрытым синонимом позиционного
набора на один цикл выпуска.
