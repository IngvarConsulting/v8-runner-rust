---
id: INV.CLI.RESTORE-TARGET-IS-OBSERVED-TWICE
check:
  - tests/cli_infobase.rs::restore_creates_an_absent_infobase_through_designer
  - tests/cli_infobase.rs::restore_replaces_an_existing_infobase_through_designer
  - tests/cli_infobase_cross_platform.rs::designer_dt_restore_authenticates_only_existing_targets
---

# Наличие цели проверяется до выбора исполнителя и повторно под блокировкой

Файловая цель опознаётся по файлу базы данных; проверка идёт до выбора исполнителя и повторяется под блокировкой, потому что между ними цель могла измениться.

Для отсутствующей файловой цели Designer запускает `/RestoreIB` без отдельных полей
`user` и `password`: пользователи появятся после загрузки DT. При замещении существующей
файловой или серверной ИБ эти поля передаются как обычно. То же правило действует для
Designer при загрузке DT в новую файловую копию через `infobase create --from`.
