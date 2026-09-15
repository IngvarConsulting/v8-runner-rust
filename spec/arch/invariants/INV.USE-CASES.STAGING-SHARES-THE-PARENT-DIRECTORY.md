---
id: INV.USE-CASES.STAGING-SHARES-THE-PARENT-DIRECTORY
status: active
governs: product
decision: DEC.2026-04-21.FULL-REPLACEMENT-PUBLISHES-THROUGH-STAGING
check: src/use_cases/staged_publication.rs::a_staging_path_shares_the_parent_directory_of_its_target
scope: [use-cases]
---

# Промежуточный каталог лежит рядом с целью

Промежуточный результат размещается в одном родительском каталоге с целью, чтобы публикация была переименованием, а не копированием через границу файловой системы.
