---
id: INV.USE-CASES.THE-VERSION-FILE-STAYS-OUT-OF-VERSION-CONTROL
check:
  - src/use_cases/ignored_files.rs::writes_every_pattern_into_a_new_gitignore
  - src/use_cases/ignored_files.rs::a_second_run_adds_nothing
  - tests/cli_config_init.rs::init_ignores_project_local_files_once
  - tests/cli_dump.rs::a_pull_refuses_when_the_version_file_is_tracked_by_git
  - tests/cli_dump.rs::a_pull_proceeds_when_the_version_file_is_not_tracked
  - tests/cli_dump.rs::a_pull_proceeds_silently_when_tracking_is_unknown
  - tests/cli_build.rs::a_push_refuses_when_the_version_file_is_tracked_by_git
---

# Файлу версий нет места в системе контроля версий

`ConfigDumpInfo.xml` — опись состояния одной базы, а не исходный код. `init` и `clone` пишут
его в игнор вместе с местным слоем, а найдя файл в индексе, раннер останавливается.

Останавливаются `pull` и `push`, которые опись пишут: до запуска платформы, отказом рода
`validation`, с путём файла и рецептом `git rm --cached`. Превью отказывает так же. Если
гит не ответил — его нет, каталог вне рабочей копии, он вернул ошибку, — работа идёт молча.
