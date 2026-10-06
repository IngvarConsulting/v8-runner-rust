---
id: INV.CLI.PREVIEW-TAKES-NO-LOCK
check:
  - tests/cli_dump.rs::dry_run_neither_takes_nor_waits_for_the_workspace_lock
  - tests/cli_infobase_lock.rs::a_preview_on_a_held_base_takes_no_base_lock
---

# Превью не берёт замков и не ждёт их

Команда в режиме превью не берёт блокировку и не встаёт в очередь за ней даже при занятом каталоге.
Замок файловой базы она тоже не берёт: превью на занятой базе проходит.
