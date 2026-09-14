---
id: INV.CLI.RESTORE-TARGET-IS-OBSERVED-TWICE
status: active
governs: product
decision: DEC.2026-09-11.RESTORE-REQUIRES-AN-EXPLICIT-TARGET-MODE
check: [tests/cli_infobase.rs::restore_creates_an_absent_infobase_through_designer, tests/cli_infobase.rs::restore_replaces_an_existing_infobase_through_designer]
scope: [cli]
---

# Наличие цели проверяется до выбора исполнителя и повторно под блокировкой

Файловая цель опознаётся по файлу базы данных; проверка идёт до выбора исполнителя и повторяется под блокировкой, потому что между ними цель могла измениться.
