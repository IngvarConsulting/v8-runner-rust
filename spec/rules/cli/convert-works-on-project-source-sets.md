---
id: INV.CLI.CONVERT-WORKS-ON-PROJECT-SOURCE-SETS
check:
  - tests/cli_convert.rs::convert_without_source_set_processes_all_source_sets_into_work_path_out
  - tests/cli_convert.rs::convert_unknown_source_set_json_keeps_convert_command_identity_before_workspace_lock
  - tests/cli_convert.rs::convert_output_root_rejects_source_overlap_before_workspace_lock
---

# `convert` работает над наборами проекта

Без указания набора обрабатываются все наборы в порядке настроек, с указанием — один
названный; неизвестное имя даёт отказ до замка. Информационная база команде не нужна.

`--output` задаёт только корень результата и проверяется на пересечение с исходниками,
базовым и рабочим каталогами.

Как называется набор и что ещё принимает команда, держит
`INV.CLI.CONVERT-TAKES-A-SET-OR-A-PACKAGE-AND-A-DIRECTION`.
