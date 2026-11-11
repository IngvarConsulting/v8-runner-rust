---
id: INV.USE-CASES.THE-WORK-PATH-IGNORES-ITSELF
check:
  - src/support/local_dir.rs::the_work_path_and_a_build_parent_it_creates_ignore_themselves
  - src/support/local_dir.rs::an_existing_build_parent_is_left_alone
  - src/use_cases/workspace_lock.rs::the_workspace_lock_creates_the_work_path_with_its_gitignore
  - tests/cli_push_generation.rs::the_first_command_leaves_no_untracked_runner_files_for_git
---

# Каталог `workPath` прячет себя от системы контроля версий

Первый писатель в `workPath` — замок, проверка настроек или журнал команды — заводит каталог
вместе с `.gitignore` со строкой `*`. Его кладёт тот же владелец, что у `localPath`, —
`prepare_ignored_dir`
(`INV.USE-CASES.THE-LOCAL-DIRECTORY-IGNORES-ITSELF`). Родителя `.build`, которого ещё не было,
раннер заводит так же; существующий `.build` и любой `.gitignore`, который уже лежит в
каталоге, не трогаются. Корневой `.gitignore` проекта раннер не меняет: после первой команды
`git status` не видит неотслеживаемых файлов `.build` и `.local`.
