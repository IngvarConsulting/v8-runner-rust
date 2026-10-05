---
id: INV.USE-CASES.A-RECORD-OF-A-RUNNING-OR-REMOTE-OWNER-IS-KEPT
check:
  - src/support/fs.rs::a_marked_record_of_a_running_owner_is_not_replaced
  - src/support/fs.rs::a_marked_record_from_another_host_is_not_replaced
  - src/support/fs.rs::a_marked_record_without_a_host_is_not_replaced
  - src/support/fs.rs::a_marked_record_of_a_killed_unreaped_owner_is_replaced
  - src/support/machine.rs::a_killed_unreaped_child_is_not_running
---

# Запись работающего или удалённого владельца остаётся на месте

Следующий процесс не заменяет запись о владельце замка, если процесс владельца на этой
машине ещё работает или запись сделана на другой машине, даже когда замок ОС ему достался:
файловая система могла этот замок не соблюсти, а номер процесса — перейти к другому. Запись
без имени машины считается сделанной на другой машине, если у этой машины имя есть.
Такой процесс сразу получает отказ с номером процесса владельца и просьбой убрать файл
вручную, а не ждёт. Убитый процесс, которого родитель ещё не дождался, уже не работает.
