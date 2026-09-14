---
id: INV.CLI.RESTORE-TARGET-IS-OBSERVED-TWICE
status: planned
governs: product
decision: DEC.2026-09-11.RESTORE-REQUIRES-AN-EXPLICIT-TARGET-MODE
check: null
scope: [cli]
---

# Наличие цели проверяется до выбора исполнителя и повторно под блокировкой

Файловая цель опознаётся по файлу базы данных; проверка идёт до выбора исполнителя и повторяется под блокировкой, потому что между ними цель могла измениться.
