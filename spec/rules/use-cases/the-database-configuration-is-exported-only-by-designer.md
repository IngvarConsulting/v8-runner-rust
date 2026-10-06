---
id: INV.USE-CASES.THE-DATABASE-CONFIGURATION-IS-EXPORTED-ONLY-BY-DESIGNER
check:
  - tests/cli_infobase.rs::download_state_db_selects_designer_by_default
  - tests/cli_infobase.rs::download_state_db_does_not_fall_back_to_another_executor
  - tests/cli_infobase.rs::download_state_db_refuses_a_providers_key_naming_another_executor
  - tests/cli_agent_scenarios.rs::configuration_export_through_the_agent_handles_working_state_only
  - tests/cli_agent_standalone.rs::a_download_of_the_database_configuration_is_refused_before_the_gate
---

# Конфигурацию базы данных выгружает только Конфигуратор

`download --state db` исполняет только `designer` (`/DumpDBCfg`). Прочие исполнители
цепочки умолчаний `download` его не пробуют и в квитанции пропущенными не названы: не готов
Конфигуратор — выбор отказывает, называя пропущенным его одного.

Ключ `providers.download`, назначивший `--state db` другого исполнителя, отказывает при
выборе исполнителя — в превью и в работе одинаково, до запуска платформы и до сессии
агента. Отказ рода `capability` называет ключ, файл, где он объявлен, и назначенного
исполнителя. Тем же родом отказывает цель, в цепочке `download` которой Конфигуратора нет.
