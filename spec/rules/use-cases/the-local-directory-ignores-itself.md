---
id: INV.USE-CASES.THE-LOCAL-DIRECTORY-IGNORES-ITSELF
check:
  - src/support/local_dir.rs::the_local_directory_is_created_with_a_gitignore_of_everything
  - src/support/local_dir.rs::a_gitignore_already_there_is_not_overwritten
  - src/support/local_dir.rs::a_writer_outside_the_local_directory_creates_nothing_there
  - src/platform/service_record.rs::a_stand_service_prepares_the_local_directory_and_nothing_else
  - src/use_cases/transport.rs::a_stand_command_takes_the_local_lock_after_the_work_path_lock
---

# Каталог `localPath` прячет себя от системы контроля версий

`localPath` заводит один владелец, `LocalDir::prepare`: он создаёт каталог и кладёт в него
`.gitignore` со строкой `*`. Чужой `.gitignore`, который уже лежит там, не перезаписывается.
Его зовёт любой первый писатель в `localPath`: служба стенда, замок команды стенда, замок и
метка файловой базы внутри него. Превью не пишет ничего. Своего `.gitignore` у `managed/`
больше нет.

В `.gitignore` проекта раннер ради `localPath` ничего не добавляет.
