---
id: INV.CLI.STATUS-WITHOUT-DEEP-STARTS-NO-PLATFORM
status: planned
governs: product
decision: DEC.2026-09-21.STATUS-ANSWERS-FROM-MEMORY
check: null
scope: [cli, use-cases]
---

# `status` без `--deep` не запускает платформу

`status` и `status --all` отвечают по памяти под `workPath`: ни одна утилита платформы
не запускается, и отсутствие платформы на машине отказом не является.
