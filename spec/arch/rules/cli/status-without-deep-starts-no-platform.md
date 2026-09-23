---
id: INV.CLI.STATUS-WITHOUT-DEEP-STARTS-NO-PLATFORM
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/215
---

# `status` без `--deep` не запускает платформу

`status` и `status --all` отвечают по памяти под `workPath`: ни одна утилита платформы
не запускается, и отсутствие платформы на машине отказом не является.
