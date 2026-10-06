---
id: INV.CLI.STATUS-WITHOUT-DEEP-STARTS-NO-PLATFORM
check:
  - tests/cli_status.rs::status_without_deep_starts_no_platform
---

# `status` без `--deep` не запускает платформу

`status` и `status --all` отвечают по памяти под `workPath`: ни одна утилита платформы
не запускается, и отсутствие платформы на машине отказом не является.
