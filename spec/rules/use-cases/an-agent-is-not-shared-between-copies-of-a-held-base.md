---
id: INV.USE-CASES.AN-AGENT-IS-NOT-SHARED-BETWEEN-COPIES-OF-A-HELD-BASE
check:
  - tests/cli_build_agent.rs::a_managed_build_loads_and_updates_in_one_session_and_records_the_generation
  - tests/cli_infobase_owner.rs::a_write_on_a_base_of_another_copy_is_refused_and_names_the_owner
---

# Агент не делится между рабочими копиями базы, у которой есть владелец

Сессия агента Конфигуратора — на время команды или долгоживущая — не обслуживает разные
рабочие копии базы, у которой есть владелец
(`INV.USE-CASES.A-DEVELOPMENT-BASE-IS-HELD-BY-ONE-WORKING-COPY`). Делить агента между
копиями можно только на базе, объявленной общей (`shared: true`).
