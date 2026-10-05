---
id: INV.USE-CASES.A-LOCK-FILE-IS-REMOVED-ONLY-BY-ITS-HOLDER
check:
  - src/support/fs.rs::a_lock_taken_on_a_removed_system_file_does_not_admit_a_second_holder
---

# Файлы замка убирает только его держатель

Файлы замка удаляет тот, кто держит замок, и до того, как его отпустить. Кто открыл
удалённый файл раньше, снова открывает файл под тем же именем и замок вместе с держателем
не получает.
