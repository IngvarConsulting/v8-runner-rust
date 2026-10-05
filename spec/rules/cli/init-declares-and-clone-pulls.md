---
id: INV.CLI.INIT-DECLARES-AND-CLONE-PULLS
check:
  - tests/cli_config_init.rs::config_init_creates_yaml_with_detected_designer_sources
  - tests/cli_config_init.rs::init_writes_the_address_named_by_the_global_key_into_origin
  - tests/cli_config_init.rs::an_existing_origin_is_not_replaced_and_the_refusal_names_the_key_that_was_used
  - tests/cli_bootstrap.rs::clone_refuses_a_non_empty_directory_before_writing_anything
  - tests/cli_bootstrap.rs::clone_refuses_a_work_path_holding_a_foreign_file
  - tests/cli_bootstrap.rs::clone_into_a_directory_holding_only_git_writes_the_project
  - tests/cli_bootstrap.rs::clone_force_writes_the_project_into_a_non_empty_directory
---

# `init` объявляет проект, `clone` его выгружает

`init` заводит проект здесь, наружу не выходит и базу не трогает. В каталоге без
проектного файла он находит наборы исходников, пишет проектный файл и местный слой и по
умолчанию объявляет `origin` файловой базой внутри рабочего каталога; с названным адресом
записывает его, а если местный слой уже объявляет другой `origin`, отказывает.

`clone` — это объявление и выгрузка одной командой в пустом каталоге. Пустым считается
каталог, где нет ничего, кроме `.git`. Рабочий каталог нового проекта не в счёт, только
пока в нём лежат лишь файлы замка и каталог журналов `logs/`, чьё содержимое не
проверяется; любой другой файл или каталог в нём делает каталог непустым. Непустой
каталог даёт отказ проверки до записи чего-либо и до запуска платформы, превью отказывает
так же. Отказ спрашивается после
замка: занятый рабочий каталог отвечает своим отказом раньше. `--force` снимает отказ по
непустому каталогу. Базу не создаёт ни одна из двух: это `infobase create`.
