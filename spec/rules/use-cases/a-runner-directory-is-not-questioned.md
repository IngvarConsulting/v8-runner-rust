---
id: INV.USE-CASES.A-RUNNER-DIRECTORY-IS-NOT-QUESTIONED
check:
  - src/use_cases/destruction_guard.rs::a_runner_owned_directory_is_never_questioned
  - src/use_cases/dump_config.rs::the_edt_designer_snapshot_is_runner_owned
  - tests/cli_convert.rs::convert_without_source_set_processes_all_source_sets_into_work_path_out
---

# Каталог раннера сторож не спрашивает

Каталог, который раннер завёл сам, заменяется без вопроса к системе контроля версий:
служебный снимок Конфигуратора у проекта EDT и вывод `convert` по умолчанию под `workPath`.
