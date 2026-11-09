---
id: INV.PLATFORM.A-PROCESS-ARGUMENT-NAMES-A-PATH-WITHOUT-THE-VERBATIM-PREFIX
check:
  - src/support/path.rs::a_process_argument_names_a_path_without_the_verbatim_prefix
  - src/support/path.rs::a_volume_path_cannot_reach_a_process
  - src/platform/process.rs::a_process_receives_paths_without_the_verbatim_prefix
  - src/platform/interactive.rs::an_interactive_process_receives_paths_without_the_verbatim_prefix
  - src/platform/service.rs::a_service_receives_paths_without_the_verbatim_prefix
  - src/platform/designer.rs::full_comparison_passes_windows_paths_without_verbatim_prefix
  - src/platform/designer.rs::cf_exports_pass_windows_drive_paths_without_verbatim_prefix
  - src/platform/designer.rs::dt_export_passes_windows_unc_path_without_verbatim_prefix
  - src/platform/ibcmd.rs::config_save_passes_windows_paths_without_verbatim_prefix
  - tests/architecture_guardrails.rs::a_process_argument_loses_the_verbatim_prefix_only_at_the_launch
---

# Процесс получает путь без приставки `\\?\`

На Windows раннер передаёт процессу каждый аргумент и рабочий каталог без приставки `\\?\` у
путей в них: `\\?\E:\x.cf` уходит как `E:\x.cf`, `\\?\UNC\srv\share\x` — как
`\\srv\share\x`. Это касается и пути, который входит в значение, например `File="\\?\E:\ib"`.
Конфигуратор и `ibcmd` путь с приставкой не находят: `/DumpDBCfg` отвечает «Файл не обнаружен»,
а канонический путь раннера приставку несёт.

Приставку снимает одно место — запуск процесса: обычный, интерактивный и запуск службы. Адаптеры
платформы её не трогают. Путь, который без приставки не выразить (`\\?\Volume{…}\`), процесс не
получает: раннер отказывает до запуска с кодом `invalid_argument` и называет этот путь, но не
весь аргумент, потому что в том же аргументе бывает пароль строки соединения. Так же отказывает и
аргумент, где такой текст встретился не как путь, например в `/C` или `/Execute`. Рабочий каталог
не в UTF-8 уходит как есть: приставку в нём текстом не распознать. Вне Windows `\\?\` — допустимая
часть имени файла, и аргумент уходит как есть.
