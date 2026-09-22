---
id: INV.CLI.LAUNCH-PREVIEW-NAMES-PROGRAM-AND-ARGS
status: active
governs: product
decision: DEC.2026-09-22.PREVIEW-LOGGING-FOLLOWS-THE-COMMAND-CONTRACT
check: tests/cli_launch.rs::launch_dry_run_json_names_the_program_and_the_arguments_it_would_run
scope: [cli]
---

# Превью запуска называет программу и аргументы

Превью запуска клиента показывает выбранный бинарник и составленную строку аргументов — то же, что ушло бы в запуск.
