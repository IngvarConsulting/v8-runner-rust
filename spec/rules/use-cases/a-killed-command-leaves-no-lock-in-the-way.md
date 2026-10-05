---
id: INV.USE-CASES.A-KILLED-COMMAND-LEAVES-NO-LOCK-IN-THE-WAY
check:
  - tests/cli_pull_memory.rs::a_pull_after_a_killed_pull_succeeds_and_removes_the_left_lock_files
  - src/support/fs.rs::files_left_by_a_killed_owner_do_not_block_and_are_removed
---

# Файлы замка убитой команды следующей не мешают

После `kill -9` ОС снимает замок, а его файлы остаются. Следующая команда берёт замок как
обычно, запись о владельце, сделанную под этим замком, заменяет своей и, закончив, убирает
файлы. Запись без такой отметки оставлена прежней версией раннера и по-прежнему означает
отказ до ручной чистки.
