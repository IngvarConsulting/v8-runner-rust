---
id: INV.CLI.STATUS-WITHOUT-DEEP-STARTS-NO-PLATFORM
check:
  - tests/cli_status.rs::status_without_deep_starts_no_platform
  - tests/cli_infobase_owner.rs::status_without_deep_names_the_live_holders_without_the_platform
---

# `status` без `--deep` не запускает платформу

`status` и `status --all` отвечают по памяти под `workPath` и, у файловой базы, по метке
владельца рядом с ней: ни одна утилита платформы не запускается, и отсутствие платформы на
машине отказом не является.
