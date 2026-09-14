---
id: INV.CLI.SIDECAR-FAILURE-DOES-NOT-RELEASE-THE-LOCK
status: planned
governs: product
decision: DEC.2026-04-20.THE-OS-LOCK-IS-THE-TRUTH-THE-SIDECAR-IS-DIAGNOSTICS
check: null
scope: [cli]
---

# Ошибка записи метаданных не снимает блокировку

Невозможность записать диагностический файл не отменяет владение каталогом и не разрешает параллельный запуск.
