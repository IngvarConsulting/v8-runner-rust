---
id: INV.USE-CASES.A-REPLACED-TREE-IS-REMOVED-IN-THE-BACKGROUND
check:
  - tests/cli_pull_memory.rs::a_full_pull_answers_before_the_replaced_tree_is_removed
  - tests/cli_pull_memory.rs::the_next_pull_removes_a_replaced_tree_the_background_left
  - src/use_cases/dump_config.rs::cleanup_orphan_dirs_removes_a_fresh_superseded_backup
  - src/use_cases/dump_config.rs::cleanup_orphan_dirs_removes_the_metadata_of_a_removed_superseded_backup
  - src/use_cases/dump_config.rs::cleanup_orphan_dirs_returns_the_backup_when_the_target_is_gone
  - src/use_cases/dump_config.rs::a_failed_dump_after_a_killed_replacement_leaves_the_returned_directory
  - src/use_cases/staged_publication.rs::background_removal_outside_the_runner_removes_the_previous_tree_in_place
  - src/support/fs.rs::a_swap_leaves_a_superseded_backup_that_only_the_marked_removal_takes
  - src/platform/backup_removal.rs::a_process_that_is_not_the_runner_does_not_spawn_the_removal
---

# Прежний каталог полной выгрузки удаляется в фоне

`pull` со ступенчатой публикацией (`pull --force` и выгрузка EDT) отвечает, не дожидаясь
удаления прежнего каталога набора. В фон уходит только удаление после успешной подмены:
пока подмена не закончена, резервная копия — единственная копия каталога, и сбой или
оборванный процесс возвращает её на место, как и раньше. Отмена до подмены ведёт себя как
прежде.

После подмены резервная копия `.dump-backup-<прогон>` помечена `superseded`, а файл
`.gitignore` в ней прячет её от git до удаления. Её удаляет отделённый процесс раннера —
скрытая подкоманда `remove-backup` с пониженным приоритетом, запущенная так же, как служба
стенда, но без журнала: его отказ виден по оставшейся копии. Процесс переживает ответ
команды. Подкоманда удаляет только помеченную резервную копию раннера рядом с её целью. Если
процесс не запустился, каталог удаляется на месте до ответа.

Не успевший или упавший процесс ничего не ломает: помеченную копию и её оставшиеся
метаданные уборка остатков следующего `pull` той же цели удаляет в любом возрасте, под
замком цели; не удалось — предупреждение в журнале, а не отказ выгрузки. Команды, которые
копию не трогают, фонового удаления не ждут.
