---
id: INV.USE-CASES.THE-DATABASE-CONFIGURATION-IS-NOT-EXPORTED-BY-THE-AGENT
check:
  - tests/cli_infobase.rs::download_state_db_selects_designer_by_default
  - tests/cli_infobase.rs::download_state_db_goes_through_ibcmd_without_designer
  - tests/cli_infobase.rs::complete_server_dbms_contract_is_dispatched_to_ibcmd
  - tests/cli_infobase.rs::download_state_db_follows_a_providers_key_naming_designer_or_ibcmd
  - tests/cli_infobase.rs::download_state_db_refuses_the_agent_before_the_platform
  - src/use_cases/infobase_export.rs::a_provider_prepared_for_another_state_is_not_dispatched
  - tests/cli_agent_scenarios.rs::configuration_export_through_the_agent_handles_working_state_only
  - tests/cli_agent_standalone.rs::a_download_of_the_database_configuration_is_refused_before_the_gate
---

# Конфигурацию базы данных выгружают Конфигуратор и `ibcmd`, агент — нет

`download --state db` исполняет `designer` (`/DumpDBCfg`) или `ibcmd` (`config save --db`).
Цепочка умолчаний пробует их в своём порядке: не готов Конфигуратор — берётся `ibcmd`, и
квитанция называет Конфигуратор пропущенным. Ключ `providers.download` может назначить
любого из них. Кто выгружает конфигурацию базы данных, говорит `src/domain/capability.rs`.

Агенту `--state db` не достаётся: у него нет команды для конфигурации базы данных. Ключ
`providers.download: agent` и цель, в цепочке `download` которой есть только агент
(автономный сервер), отказывают при выборе исполнителя — в превью и в работе одинаково, до
запуска платформы и до сессии агента. Квитанция называет агента пропущенным с причиной, а
исполнитель, выбранный под рабочее состояние, конфигурацию базы данных не получает. Отказ
рода `capability` называет причину и выход: у ключа — файл, где он объявлен, и
исполнителей, которых можно назначить; у такой цели — выгрузку рабочей конфигурации без
`--state db`.
