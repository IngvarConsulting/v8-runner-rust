---
id: INV.CLI.SIDECAR-FAILURE-DOES-NOT-RELEASE-THE-LOCK
status: active
governs: product
decision: DEC.2026-04-20.THE-OS-LOCK-IS-THE-TRUTH-THE-SIDECAR-IS-DIAGNOSTICS
check: src/use_cases/workspace_lock.rs::an_unwritable_sidecar_does_not_release_the_lock
scope: [cli]
---

# Ошибка записи метаданных не снимает блокировку

Невозможность записать диагностический файл не отменяет владение каталогом и не разрешает параллельный запуск.
