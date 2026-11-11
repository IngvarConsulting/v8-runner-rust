---
id: INV.USE-CASES.CLEANUP-TOUCHES-ONLY-ITS-OWN-ARTEFACTS
check:
  - src/use_cases/dump_config.rs::cleanup_orphan_dirs_removes_old_valid_metadata
  - src/use_cases/dump_config.rs::cleanup_orphan_dirs_ignores_recent_metadata
  - src/use_cases/dump_config.rs::cleanup_orphan_dirs_returns_the_backup_when_the_target_is_gone
  - src/use_cases/dump_config.rs::cleanup_orphan_dirs_does_not_return_a_foreign_backup
  - src/use_cases/dump_config.rs::cleanup_orphan_dirs_does_not_return_a_superseded_backup
  - src/use_cases/dump_config.rs::a_failed_dump_after_a_killed_replacement_leaves_the_returned_directory
  - src/use_cases/dump_config.rs::cleanup_orphan_dirs_removes_a_stale_backup_beside_its_target
  - src/use_cases/staged_publication.rs::cleanup_keeps_a_stale_backup_when_its_target_is_gone
  - src/use_cases/staged_publication.rs::cleanup_looks_at_the_target_named_by_the_backup
  - src/use_cases/staged_publication.rs::cleanup_removes_a_fresh_superseded_backup
  - src/use_cases/dump_config.rs::cleanup_orphan_dirs_removes_a_fresh_superseded_backup
  - src/use_cases/dump_config.rs::cleanup_orphan_dirs_ignores_foreign_metadata
  - src/use_cases/dump_config.rs::cleanup_orphan_dirs_ignores_metadata_of_another_target
  - src/use_cases/dump_config.rs::cleanup_orphan_dirs_ignores_malformed_metadata
  - src/use_cases/dump_config.rs::cleanup_orphan_dirs_ignores_a_directory_named_outside_the_contract
  - src/use_cases/artifacts.rs::cleanup_orphan_files_removes_old_stage_directory_cleanup_unit
  - src/use_cases/artifacts.rs::cleanup_orphan_files_ignores_recent_metadata
  - src/use_cases/artifacts.rs::cleanup_orphan_files_ignores_foreign_metadata
  - src/use_cases/artifacts.rs::cleanup_orphan_files_ignores_malformed_metadata
  - src/use_cases/infobase_export.rs::orphan_cleanup_removes_only_owned_stale_export_files
  - src/use_cases/staged_publication.rs::orphan_cleanup_requires_exact_target_kind_and_run_name_contract
  - src/use_cases/throwaway_infobase.rs::orphan_cleanup_removes_only_stale_own_throwaway_bases
  - tests/cli_convert.rs::convert_cleans_stale_throwaway_bases_and_keeps_the_build_infobase
---

# Уборка трогает только собственные следы

Уборка удаляет устаревшие промежуточные и резервные копии — файлы и каталоги — и
временные базы разбора пакетов,
опознанные как свои по собственным метаданным и имени. Свежий свой след она оставляет, а
чужой или нечитаемый рядом с целью не трогает.

База сборки под `workPath/temp/build-infobase/` сохраняется после уборки.

Резервная копия при отсутствующей цели — след гибели процесса между заменой и единственная
копия, поэтому в любом возрасте она не удаляется. `pull` под замком цели возвращает свою
резервную копию на место раньше, чем что-либо создаст цель: иначе пустой или частичный каталог
выдал бы её за обычный след. Если вернуть не удалось или цель не удалось осмотреть, команда
отказывает. Копия, оставшаяся от завершённой замены, помечена и не возвращается: она старше цели. Такую
копию уборка удаляет в любом возрасте — её не успело удалить фоновое удаление
(`INV.USE-CASES.A-REPLACED-TREE-IS-REMOVED-IN-THE-BACKGROUND`).
Уборка других команд резервную копию при цели, которую не удалось осмотреть, не удаляет.
