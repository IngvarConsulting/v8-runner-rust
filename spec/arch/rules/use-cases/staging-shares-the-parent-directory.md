---
id: INV.USE-CASES.STAGING-SHARES-THE-PARENT-DIRECTORY
check: [src/use_cases/staged_publication.rs::a_staging_path_shares_the_parent_directory_of_its_target]
---

# Промежуточный каталог лежит рядом с целью

Промежуточный результат размещается в одном родительском каталоге с целью, чтобы публикация была переименованием, а не копированием через границу файловой системы.
