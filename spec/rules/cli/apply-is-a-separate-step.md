---
id: INV.CLI.APPLY-IS-A-SEPARATE-STEP
check:
  - tests/cli_apply.rs::a_push_without_apply_loads_and_apply_applies_it
  - tests/cli_apply.rs::an_apply_of_one_set_applies_only_that_set
  - tests/cli_apply.rs::providers_apply_assigns_the_executor_of_the_apply
  - src/use_cases/build_project.rs::a_push_without_apply_loads_and_never_calls_update_db_cfg
  - src/use_cases/build_project.rs::an_ibcmd_push_without_apply_imports_and_never_calls_config_apply
  - src/use_cases/build_project.rs::a_push_without_apply_leaves_the_tool_extension_unapplied
  - src/use_cases/apply.rs::an_apply_walks_the_sets_in_order_and_skips_external_files
  - tests/cli_build_agent.rs::an_agent_push_without_apply_leaves_the_update_to_apply
---

# Применение — отдельный шаг, а не хвост отправки

Накатка — два акта платформы: загрузка в основную конфигурацию и применение к конфигурации
базы данных. `push` делает оба; `push --no-apply` останавливается на первом, и база данных
остаётся прежней при работающих сеансах; `apply` приводит базу данных к основной
конфигурации. Правило одно у всех исполнителей. Состояний внутри базы
три: «совпадают», «есть непринятое», «обновлено динамически».

Чужими сеансами при применении управляет `INV.CLI.APPLY-SESSIONS-GOVERNS-FOREIGN-SESSIONS`.
