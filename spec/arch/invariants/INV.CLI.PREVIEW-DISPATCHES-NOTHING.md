---
id: INV.CLI.PREVIEW-DISPATCHES-NOTHING
status: active
governs: product
decision: DEC.2026-09-11.PREVIEW-STOPS-BEFORE-THE-PROVIDER-IS-DISPATCHED
check: [tests/cli_build.rs::build_dry_run_plans_every_source_set_without_dispatching_designer, tests/cli_artifacts.rs::artifacts_dry_run_plans_the_package_without_building_it, tests/cli_convert.rs::convert_dry_run_plans_every_source_set_without_dispatching_the_edt_cli]
scope: [cli]
---

# Превью не запускает исполнителя

В режиме превью ни одна команда не доходит до запуска процесса платформы и не создаёт целей.
