---
id: INV.USE-CASES.A-VERSION-FILE-REPLACEMENT-LEAVES-NO-TRACE
check:
  - src/support/fs.rs::an_atomic_write_keeps_the_mode_and_leaves_no_candidate
  - src/support/fs.rs::an_atomic_write_of_a_new_file_follows_the_umask
  - src/support/fs.rs::an_atomic_write_refuses_a_path_without_a_file_name
  - src/use_cases/version_file.rs::temporary_files_left_by_a_killed_write_are_removed
  - src/change_detection/scanner.rs::service_and_generated_paths_are_never_scanned
---

# Замена копии описи не оставляет следов

Копию описи раннера, тождество её пары и свидетельство каталога раннер заменяет атомарно:
читатель видит прежний файл или новый целиком. Новая опись из стадии ложится на место копии
переименованием, а с другого тома — атомарной копией. Временный файл замены лежит рядом с целью и
назван по ней (`<имя>.candidate-…`); оставленный снятым процессом убирается при следующей записи
копии. В каталог набора раннер описи и её временных файлов не пишет; файлы с этими именами,
оставшиеся там от прежних версий, в обход изменений не входят.
