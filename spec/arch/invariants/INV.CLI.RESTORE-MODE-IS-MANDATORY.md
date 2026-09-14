---
id: INV.CLI.RESTORE-MODE-IS-MANDATORY
status: active
governs: product
decision: DEC.2026-09-11.RESTORE-REQUIRES-AN-EXPLICIT-TARGET-MODE
check: tests/cli_infobase.rs::restore_without_a_target_mode_is_refused_before_provider_selection
scope: [cli]
---

# Загрузка базы без режима цели отклоняется до выбора исполнителя

Команда возврата базы без явного режима отказывает раньше, чем выбран исполнитель и запущен процесс.
