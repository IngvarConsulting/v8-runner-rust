---
id: INV.CLI.PREVIEW-TAKES-NO-LOCK
status: active
governs: product
decision: DEC.2026-09-11.PREVIEW-DOES-NOT-TAKE-THE-LOCK
check: tests/cli_dump.rs::dry_run_neither_takes_nor_waits_for_the_workspace_lock
scope: [cli]
---

# Превью не захватывает рабочий каталог и не ждёт его

Команда в режиме превью не берёт блокировку и не встаёт в очередь за ней даже при занятом каталоге.
