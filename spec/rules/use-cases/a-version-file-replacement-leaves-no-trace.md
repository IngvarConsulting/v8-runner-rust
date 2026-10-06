---
id: INV.USE-CASES.A-VERSION-FILE-REPLACEMENT-LEAVES-NO-TRACE
check:
  - src/support/fs.rs::an_atomic_write_keeps_the_mode_and_leaves_no_candidate
  - src/support/fs.rs::an_atomic_write_of_a_new_file_follows_the_umask
  - src/support/fs.rs::an_atomic_write_refuses_a_path_without_a_file_name
  - src/use_cases/version_file.rs::temporary_files_left_by_a_killed_write_are_removed
  - src/change_detection/scanner.rs::service_and_generated_paths_are_never_scanned
  - tests/cli_pull_memory.rs::a_left_temporary_version_file_does_not_stop_a_full_pull
---

# Замена файла версий не оставляет следов

Файл версий в каталоге набора и копию раннера раннер заменяет атомарно: читатель видит
прежний файл или новый целиком. Замена сохраняет права прежнего файла, а новый файл
получает права, которые дало бы обычное создание файла под тем же umask. Временный файл
замены лежит рядом с целью и назван по ней (`<имя>.candidate-…`). Оставленный снятым
процессом убирается в начале любой выгрузки и загрузки набора и при записи копии; в обход
изменений он не входит, и сторож замены каталога о нём не спрашивает.
