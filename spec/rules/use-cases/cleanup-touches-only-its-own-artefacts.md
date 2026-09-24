---
id: INV.USE-CASES.CLEANUP-TOUCHES-ONLY-ITS-OWN-ARTEFACTS
check: [src/support/fs.rs::publish_file_atomically_ignores_backup_cleanup_failure]
---

# Уборка трогает только собственные следы

Удаляются лишь промежуточные и резервные каталоги, опознанные по собственным метаданным; чужие каталоги рядом с целью не трогаются.
