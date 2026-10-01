---
id: INV.CLI.PREVIEW-DISPATCHES-NOTHING
check:
  - tests/cli_build.rs::build_dry_run_plans_every_source_set_without_dispatching_designer
  - tests/cli_artifacts.rs::artifacts_dry_run_plans_the_package_without_building_it
  - tests/cli_convert.rs::convert_dry_run_plans_every_source_set_without_dispatching_the_edt_cli
  - tests/contract_previews.rs::no_preview_creates_anything_in_the_work_path
  - tests/contract_previews.rs::no_preview_claims_that_an_executor_got_work
  - src/use_cases/convert_sources.rs::an_interrupted_preview_reports_no_work_for_the_edt_cli
  - tests/cli_build.rs::a_planned_edt_build_does_not_load_the_generated_designer_files
---

# Превью не запускает исполнителя

В режиме превью ни одна команда не доходит до запуска процесса платформы и не создаёт
целей.

Там, где выбора исполнителя нет, превью говорит об этом закрытым признаком
`provider_dispatched: false` и предметом самого глагола — набором, артефактом, режимом,
целевым путём. Словарь кандидатов с единственным элементом и зеркальные коллекции
«запланировано» против «сделано» не заводятся: поля называют то, что было бы сделано, а
получил ли исполнитель работу, говорит признак.

Предел знания превью называет отдельным значением поля — `status: planned`,
`exit_code: -1`, — а не умолчанием соседнего.
