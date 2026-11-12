---
id: INV.USE-CASES.THE-PROJECT-IGNORES-MACHINE-LOCAL-FILES
check:
  - src/use_cases/ignored_files.rs::writes_every_pattern_into_a_new_gitignore
  - src/use_cases/ignored_files.rs::a_second_run_adds_nothing
  - src/use_cases/ignored_files.rs::a_nested_config_ignores_the_lock_from_the_project_dir
  - src/use_cases/ignored_files.rs::a_project_inside_a_larger_repository_keeps_the_root_gitignore_untouched
  - src/use_cases/ignored_files.rs::a_pattern_in_info_exclude_is_still_written
  - src/use_cases/ignored_files.rs::a_lock_pattern_must_cover_both_lock_files
  - tests/cli_config_init.rs::init_ignores_project_local_files_once
  - tests/cli_config_init.rs::init_with_a_nested_config_ignores_the_lock_of_every_source_set
  - tests/cli_bootstrap.rs::clone_into_a_subdirectory_of_a_repository_writes_the_project_gitignore
---

# Проект не отдаёт в гит файлы одной машины

`init` и `clone` пишут в игнор местный слой конфига (`v8project.local.yaml`) и замок выгрузки
(`.dump-*.lock*`). Опись версий в игнор не пишется: в каталоги наборов раннер её не кладёт
(`INV.USE-CASES.THE-VERSION-FILE-LIVES-ONLY-IN-THE-RUNNER-MEMORY`).

Шаблоны пишутся в `.gitignore` каталога проекта — там, где ищутся наборы: у `init` это
каталог запуска, у `clone` — `--project-dir`; в гите и без него одинаково. Корень рабочей
копии выше проекта (монорепо, репозиторий в домашнем каталоге) раннеру не принадлежит, и
его `.gitignore` не трогается. Шаблоны без `/` действуют на любой глубине под каталогом
проекта: замок — в каталоге любого набора, местный слой — рядом с вложенным конфигом. Покрытым
шаблон считается, только если его даёт `.gitignore` внутри рабочей копии — любой, в том числе
выше проекта: игнор одной машины (`.git/info/exclude`, `core.excludesFile`) с репозиторием не
уезжает.
