---
id: INV.USE-CASES.AN-AGENT-IS-NOT-SHARED-BETWEEN-COPIES-OF-A-HELD-BASE
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/327
---

# Агент не делится между рабочими копиями базы, у которой есть владелец

Сессия агента Конфигуратора — на время команды или долгоживущая — не обслуживает разные
рабочие копии базы, у которой есть владелец
(`INV.USE-CASES.A-DEVELOPMENT-BASE-IS-HELD-BY-ONE-WORKING-COPY`). Делить агента между
копиями можно только на базе, объявленной общей (`shared: true`).
