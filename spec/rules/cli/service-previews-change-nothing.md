---
id: INV.CLI.SERVICE-PREVIEWS-CHANGE-NOTHING
check:
  - src/use_cases/managed_stand/mod.rs::a_preview_changes_nothing
---

# Превью служб ничего не меняет

`start`, `stop` и `restart` с `--dry-run` показывают план и не запускают и не останавливают процессов.
