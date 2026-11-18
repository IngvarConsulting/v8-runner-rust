---
id: INV.CLI.SERVICE-PREVIEWS-CHANGE-NOTHING
check:
  - src/use_cases/managed_stand/mod.rs::a_preview_changes_nothing
  - tests/cli_web.rs::web_preview_reports_its_apache_and_private_files_without_writing
---

# Превью служб ничего не меняет

`start`, `stop` и `restart` с `--dry-run` показывают план и не запускают и не останавливают процессов.
