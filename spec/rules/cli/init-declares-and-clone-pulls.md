---
id: INV.CLI.INIT-DECLARES-AND-CLONE-PULLS
check:
  - tests/cli_config_init.rs::config_init_creates_yaml_with_detected_designer_sources
  - tests/cli_config_init.rs::init_with_a_nested_config_ignores_the_lock_of_every_source_set
  - tests/cli_config_init.rs::init_writes_the_address_named_by_the_global_key_into_origin
  - tests/cli_config_init.rs::an_existing_origin_is_not_replaced_and_the_refusal_names_the_key_that_was_used
  - tests/cli_bootstrap.rs::clone_refuses_a_non_empty_directory_before_writing_anything
  - tests/cli_bootstrap.rs::clone_refuses_a_work_path_holding_a_foreign_file
  - tests/cli_bootstrap.rs::clone_into_a_directory_holding_only_git_writes_the_project
  - tests/cli_bootstrap.rs::clone_force_writes_the_project_into_a_non_empty_directory
  - src/use_cases/bootstrap_project.rs::a_local_path_holding_only_its_preparation_does_not_make_the_directory_non_empty
  - src/use_cases/bootstrap_project.rs::a_nested_local_path_holding_only_its_preparation_does_not_make_the_directory_non_empty
  - src/use_cases/config_init.rs::the_default_origin_lies_in_the_declared_local_path
---

# `init` объявляет проект, `clone` его выгружает

`init` заводит проект здесь, наружу не выходит и базу не трогает. В каталоге без
проектного файла он находит наборы исходников, пишет проектный файл и местный слой и по
умолчанию объявляет `origin` файловой базой `<localPath>/db` (`File=.local/db`); с названным адресом
записывает его, а если местный слой уже объявляет другой `origin`, отказывает.
Относительные пути наборов в проектном YAML используют `/` на всех ОС и считаются от
каталога этого файла, в том числе при вложенном `--output`.

`clone` — это объявление и выгрузка одной командой в пустом каталоге. Пустым считается
каталог, где нет ничего, кроме `.git`. Рабочий каталог нового проекта не в счёт, только
пока в нём лежат лишь файлы замка и каталог журналов `logs/`, чьё содержимое не
проверяется. Так же не в счёт `localPath`, пока в нём лишь `.gitignore` и файлы замков.
Любой другой файл или каталог в них делает каталог непустым. Непустой
каталог даёт отказ проверки до записи чего-либо и до запуска платформы, превью отказывает
так же. Отказ спрашивается после
замка: занятый рабочий каталог отвечает своим отказом раньше. `--force` снимает отказ по
непустому каталогу. Базу не создаёт ни одна из двух: это `infobase create`.
