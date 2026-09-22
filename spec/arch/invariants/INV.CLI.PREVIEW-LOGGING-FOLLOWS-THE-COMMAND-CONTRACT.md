---
id: INV.CLI.PREVIEW-LOGGING-FOLLOWS-THE-COMMAND-CONTRACT
status: active
governs: product
decision: DEC.2026-09-22.PREVIEW-LOGGING-FOLLOWS-THE-COMMAND-CONTRACT
check: [tests/contract_previews.rs::every_preview_leaves_a_line_in_the_action_log, tests/cli_configuration_transition.rs::apply_reset_preview_does_not_dispatch_or_create_work_path, tests/cli_configuration_transition.rs::busy_workspace_blocks_execution_but_not_read_only_preview]
scope: [cli]
---

# Превью сохраняет журналирование своей команды

У существовавших до apply/reset команд остаётся прежняя запись о превью
в журнале действий. Превью apply/reset не создаёт workPath и журнал,
не запускает платформу и не ждёт блокировку рабочего каталога.
