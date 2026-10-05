---
id: INV.USE-CASES.A-DUMP-LOCK-LEAVES-NO-FILES-IN-THE-SOURCE-TREE
check:
  - tests/cli_pull_memory.rs::a_finished_pull_leaves_no_dump_lock_beside_the_source_set
  - tests/cli_pull_memory.rs::an_interrupted_pull_leaves_no_dump_lock_beside_the_source_set
  - src/support/fs.rs::released_advisory_lock_leaves_no_files_and_can_be_reacquired
  - src/support/fs.rs::a_failed_acquisition_leaves_no_system_file
  - src/support/fs.rs::a_lock_taken_on_a_removed_system_file_does_not_admit_a_second_holder
---

# Замок выгрузки не оставляет файлов в дереве исходников

Замок выгрузки стоит рядом с целью, чтобы его видели и команды с другим `workPath`. Его
файлы `.dump-<hash>.lock*` живут, пока команда держит замок: закончилась она удачно, с
отказом или по отмене — в каталоге-родителе набора их не остаётся. Файл замка удаляет
только тот, кто держит замок, и до того, как его отпустить; кто открыл удалённый файл
раньше, снова открывает файл под тем же именем и замок вдвоём не держит.
