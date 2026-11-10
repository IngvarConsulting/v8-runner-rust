---
id: INV.USE-CASES.A-DB-UPDATE-NEVER-GOES-DYNAMIC
check:
  - src/platform/designer.rs::update_db_cfg_always_forbids_dynamic_update
  - src/platform/ibcmd.rs::config_apply_always_forbids_dynamic_update
  - src/use_cases/apply/act.rs::an_agent_update_always_forbids_dynamic_update
  - tests/architecture_guardrails.rs::no_db_update_allows_dynamic_update
  - tests/cli_apply.rs::providers_apply_assigns_the_executor_of_the_apply
  - tests/cli_apply.rs::an_apply_without_a_set_uses_one_managed_agent_session
  - tests/cli_build_agent.rs::a_managed_build_loads_and_updates_in_one_session_and_records_the_generation
  - tests/cli_init.rs::init_designer_creates_infobase_and_skips_edt_workspace
---

# Обновление конфигурации базы данных никогда не бывает динамическим

Динамическое обновление конфигурации базы данных в платформе работает нестабильно, поэтому раннер его
не использует. Каждое обновление конфигурации базы данных, которое запускает раннер (`push`, `apply`,
`make`, `infobase create` и любая другая команда), явно запрещает динамическое обновление у своего
исполнителя: у пакетного Конфигуратора — `/UpdateDBCfg -Dynamic-`, у агента Конфигуратора —
`config update-db-cfg --dynamic-disable`, у `ibcmd` — `config apply --dynamic=disable`. На умолчание
платформы раннер не полагается: без явного запрета платформа разрешает динамическое обновление, если
обычное не удалось.

Если обычное обновление невозможно, например из-за открытых сеансов, команда отказывает. Базу она
динамически не обновляет.
