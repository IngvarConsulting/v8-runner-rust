---
id: INV.CLI.INIT-WORKS-IN-THE-LAUNCH-DIRECTORY-AND-A-REFUSAL-WRITES-NOTHING
check:
  - tests/cli_config_init.rs::init_run_outside_the_project_refuses_before_writing_anything
  - tests/cli_config_init.rs::init_run_outside_an_undeclared_project_refuses_before_writing_anything
  - tests/cli_config_init.rs::init_with_a_nested_output_and_no_sources_leaves_no_config_directory
  - tests/cli_config_init.rs::a_gitignore_write_failure_leaves_the_local_layer_as_it_was
  - tests/cli_config_init.rs::a_gitignore_write_failure_after_force_restores_the_project_file
  - src/use_cases/config_init.rs::an_output_outside_the_project_is_refused_before_anything_is_read_or_written
  - src/use_cases/config_init.rs::a_failed_write_restores_the_written_files_and_removes_the_created_directories
  - src/use_cases/ignored_files.rs::a_config_directory_not_yet_created_is_probed_from_the_project_dir
---

# `init` работает с проектом каталога запуска, и отказ ничего не записывает

Проект для `init` — каталог, из которого его запустили: там ищутся наборы и лежит
`.gitignore`, который он дописывает. `--output` называет путь проектного файла внутри
этого каталога, вложенный тоже. Проектный файл вне него — отказ проверки до чтения
исходников и до записи чего-либо, в объявленном проекте так же, как в новом.

Отказ `init` не оставляет изменённых файлов. Всё, что может отказать, проверяется до
записи. Если запись всё же отказала, уже записанные файлы получают прежнее содержимое,
а созданные каталоги убираются. Файл, который вернуть не удалось, называет ошибка.

Источник: [`cli-init.html#init-output-outside`](../../../docs/site/cli-init.html#init-output-outside),
[`cli-init.html#init-write-failed`](../../../docs/site/cli-init.html#init-write-failed).
