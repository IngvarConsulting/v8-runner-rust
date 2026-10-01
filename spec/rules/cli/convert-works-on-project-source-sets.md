---
id: INV.CLI.CONVERT-WORKS-ON-PROJECT-SOURCE-SETS
check:
  - tests/cli_convert.rs::convert_without_source_set_processes_all_source_sets_into_work_path_out
  - tests/cli_convert.rs::convert_unknown_source_set_json_keeps_convert_command_identity_before_workspace_lock
  - tests/cli_convert.rs::convert_output_root_rejects_source_overlap_before_workspace_lock
---

# `convert` работает над наборами проекта, а не над произвольными путями

Без `--source-set` обрабатываются все наборы в порядке настроек, с ним — один названный;
неизвестное имя даёт отказ до замка. Направление берётся из формата проекта, а не из
аргументов. Информационная база команде не нужна, и выбора исполнителя у неё нет.

Путь источника в аргументах не принимается. `--output` задаёт только корень результата и
проверяется на пересечение с исходниками, базовым и рабочим каталогами.
