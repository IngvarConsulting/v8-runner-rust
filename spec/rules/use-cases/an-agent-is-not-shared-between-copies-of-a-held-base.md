---
id: INV.USE-CASES.AN-AGENT-IS-NOT-SHARED-BETWEEN-COPIES-OF-A-HELD-BASE
check:
  - tests/cli_build_agent.rs::a_managed_build_loads_and_updates_in_one_session_and_records_the_generation
---

# Агент не делится между рабочими копиями базы, у которой есть владелец

Сессия агента Конфигуратора живёт одной командой одной рабочей копии и не обслуживает
разные рабочие копии базы, у которой есть владелец
(`INV.USE-CASES.A-DEVELOPMENT-BASE-IS-HELD-BY-ONE-WORKING-COPY`).
