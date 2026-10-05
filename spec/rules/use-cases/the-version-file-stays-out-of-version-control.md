---
id: INV.USE-CASES.THE-VERSION-FILE-STAYS-OUT-OF-VERSION-CONTROL
check:
  - src/use_cases/ignored_files.rs::writes_every_pattern_into_a_new_gitignore
  - src/use_cases/ignored_files.rs::a_second_run_adds_nothing
  - src/use_cases/ignored_files.rs::a_nested_config_ignores_the_version_file_from_the_project_dir
  - src/use_cases/ignored_files.rs::a_project_inside_a_larger_repository_keeps_the_root_gitignore_untouched
  - src/use_cases/ignored_files.rs::a_pattern_in_info_exclude_is_still_written
  - src/use_cases/ignored_files.rs::a_lock_pattern_must_cover_both_lock_files
  - tests/cli_config_init.rs::init_ignores_project_local_files_once
  - tests/cli_config_init.rs::init_with_a_nested_config_ignores_the_version_file_of_every_source_set
  - tests/cli_bootstrap.rs::clone_into_a_subdirectory_of_a_repository_writes_the_project_gitignore
  - tests/cli_dump.rs::a_pull_refuses_when_the_version_file_is_tracked_by_git
  - tests/cli_dump.rs::a_pull_proceeds_when_the_version_file_is_not_tracked
  - tests/cli_dump.rs::a_pull_proceeds_silently_when_tracking_is_unknown
  - tests/cli_build.rs::a_push_refuses_when_the_version_file_is_tracked_by_git
---

# Файлу версий нет места в системе контроля версий

`ConfigDumpInfo.xml` — опись состояния одной базы, а не исходный код. `init` и `clone` пишут
его в игнор вместе с местным слоем, а найдя файл в индексе, раннер останавливается.

Шаблоны пишутся в `.gitignore` каталога проекта — там, где ищутся наборы: у `init` это
каталог запуска, у `clone` — `--project-dir`; в гите и без него одинаково. Корень рабочей
копии выше проекта (монорепо, репозиторий в домашнем каталоге) раннеру не принадлежит, и
его `.gitignore` не трогается. Шаблоны без `/` действуют на любой глубине под каталогом
проекта: опись и замок — в каталоге любого набора, местный слой — рядом с вложенным
конфигом. Покрытым шаблон считается, только если его даёт `.gitignore` внутри рабочей
копии — любой, в том числе выше проекта: игнор одной машины (`.git/info/exclude`,
`core.excludesFile`) с репозиторием не уезжает.

Останавливаются `pull` и `push`, которые опись пишут: до запуска платформы, отказом рода
`validation`, с путём файла и рецептом `git rm --cached`. Превью отказывает так же. Если
гит не ответил — его нет, каталог вне рабочей копии, он вернул ошибку, — работа идёт молча.
