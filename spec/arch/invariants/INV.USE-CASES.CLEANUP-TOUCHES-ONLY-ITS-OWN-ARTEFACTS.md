---
id: INV.USE-CASES.CLEANUP-TOUCHES-ONLY-ITS-OWN-ARTEFACTS
status: active
governs: product
decision: DEC.2026-04-21.FULL-REPLACEMENT-PUBLISHES-THROUGH-STAGING
check: src/support/fs.rs::publish_file_atomically_ignores_backup_cleanup_failure
scope: [use-cases]
---

# Уборка трогает только собственные следы

Удаляются лишь промежуточные и резервные каталоги, опознанные по собственным метаданным; чужие каталоги рядом с целью не трогаются.
