---
id: INV.CLI.RESTORE-MODE-IS-MANDATORY
check: [tests/cli_infobase.rs::restore_without_a_target_mode_is_refused_before_provider_selection]
---

# Загрузка базы без режима цели отклоняется до выбора исполнителя

Команда возврата базы без явного режима отказывает раньше, чем выбран исполнитель и запущен процесс.
