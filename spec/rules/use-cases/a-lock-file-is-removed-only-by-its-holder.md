---
id: INV.USE-CASES.A-LOCK-FILE-IS-REMOVED-ONLY-BY-ITS-HOLDER
check:
  - src/support/fs.rs::a_lock_taken_on_a_removed_system_file_does_not_admit_a_second_holder
  - src/support/fs.rs::a_marked_record_of_a_running_owner_is_not_replaced
  - src/support/fs.rs::a_marked_record_from_another_host_is_not_replaced
---

# Файлы замка убирает только его держатель

Файлы замка удаляет тот, кто держит замок, и до того, как его отпустить. Кто открыл
удалённый файл раньше, снова открывает файл под тем же именем и замок вместе с держателем
не получает. Запись о владельце, чей процесс на этой машине ещё жив или чей процесс
выполняется на другой машине, следующий процесс не заменяет, даже если замок ОС ему
достался: файловая система могла этот замок не соблюсти.
