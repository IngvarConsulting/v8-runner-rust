---
id: INV.USE-CASES.CLEANUP-TOUCHES-ONLY-ITS-OWN-ARTEFACTS
check:
  - src/use_cases/dump_config.rs::cleanup_orphan_dirs_removes_old_valid_metadata
  - src/use_cases/dump_config.rs::cleanup_orphan_dirs_ignores_recent_metadata
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
---

# Уборка трогает только собственные следы

Уборка удаляет устаревшие промежуточные и резервные копии — файлы и каталоги, —
опознанные как свои по собственным метаданным и имени. Свежий свой след она оставляет, а
чужой или нечитаемый рядом с целью не трогает.
