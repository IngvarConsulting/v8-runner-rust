---
id: INV.CLI.PREVIEW-REFUSES-CLEANING-THE-WORKPATH
status: active
governs: product
decision: DEC.2026-09-11.PREVIEW-DOES-NOT-TAKE-THE-LOCK
check: [tests/cli_dump.rs::dry_run_refuses_clean_before_execution_instead_of_skipping_it, tests/cli_bootstrap.rs::mcp_rejects_clean_before_execution_flag]
scope: [cli, mcp]
---

# Очистка рабочего каталога с превью отклоняется, а не пропускается

Совмещение превью с очисткой `workPath` даёт отказ: молчаливый пропуск обещал бы одно, а делал другое.
