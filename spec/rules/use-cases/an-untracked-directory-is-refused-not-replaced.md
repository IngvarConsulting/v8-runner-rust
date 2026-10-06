---
id: INV.USE-CASES.AN-UNTRACKED-DIRECTORY-IS-REFUSED-NOT-REPLACED
check:
  - src/use_cases/destruction_guard.rs::without_an_answer_every_file_is_a_loss_and_the_work_is_refused
  - src/platform/git.rs::a_failing_git_inside_a_worktree_names_its_reason
  - src/use_cases/destruction_guard.rs::a_failing_git_inside_a_worktree_is_not_advised_to_be_put_under_version_control
  - src/use_cases/destruction_guard.rs::an_empty_directory_without_an_answer_has_nothing_to_lose
  - src/use_cases/destruction_guard.rs::a_preview_names_the_losses_for_either_consent
  - tests/cli_dump.rs::a_directory_outside_version_control_is_refused_and_every_file_is_named
  - tests/cli_dump.rs::an_empty_directory_outside_version_control_has_nothing_to_lose
  - tests/cli_dump.rs::a_pull_preview_names_the_losses_and_touches_nothing
---

# Каталог вне системы контроля версий не заменяют, а отказывают

Третий ответ системы контроля версий — «ответа нет»: гита нет, каталог не репозиторий,
вызов не удался — даёт отказ на тех же правах, что найденное безвозвратное. Каталог,
который под контролем версий не находится, считается разрушением целиком: отказ
перечисляет каждый файл в нём, кроме тех, что работа пишет заново в его корне. Пустой
каталог терять нечего, и работа идёт. Перезапись файла, чьё содержимое зафиксировано,
разрушением не считается.

Отказ называет причину словами гита и различает каталог вне рабочей копии и гит, который не
ответил внутри неё: взять каталог под контроль версий советуют только первому, второму —
устранить то, что гит назвал.

Явный выход один — `--force`: что он называет в ответе, держит правило о `--force`. Превью
(`--dry-run`) перечисляет потери поимённо и ничего не трогает: без согласия — то, на чём
работа остановится, с согласием — то, что она уничтожит.
