---
id: INV.CLI.PREVIEW-DISPATCHES-NOTHING
check:
  - tests/cli_build.rs::build_dry_run_plans_every_source_set_without_dispatching_designer
  - tests/cli_artifacts.rs::artifacts_dry_run_plans_the_package_without_building_it
  - tests/cli_convert.rs::convert_dry_run_plans_every_source_set_without_dispatching_the_edt_cli
  - tests/contract_previews.rs::no_preview_creates_anything_in_the_work_path
  - tests/cli_build.rs::a_planned_edt_build_does_not_load_the_generated_designer_files
---

# Превью не запускает исполнителя

В режиме превью ни одна команда не доходит до запуска процесса платформы и не создаёт целей.
