---
id: INV.CLI.PREVIEW-TAKES-NO-LOCK
check: [tests/cli_dump.rs::dry_run_neither_takes_nor_waits_for_the_workspace_lock]
---

# Превью не захватывает рабочий каталог и не ждёт его

Команда в режиме превью не берёт блокировку и не встаёт в очередь за ней даже при занятом
каталоге.
