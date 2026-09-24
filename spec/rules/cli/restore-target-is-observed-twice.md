---
id: INV.CLI.RESTORE-TARGET-IS-OBSERVED-TWICE
check:
  - tests/cli_infobase.rs::restore_creates_an_absent_infobase_through_designer
  - tests/cli_infobase.rs::restore_replaces_an_existing_infobase_through_designer
---

# Наличие цели проверяется до выбора исполнителя и повторно под блокировкой

Файловая цель опознаётся по файлу базы данных; проверка идёт до выбора исполнителя и повторяется под блокировкой, потому что между ними цель могла измениться.
