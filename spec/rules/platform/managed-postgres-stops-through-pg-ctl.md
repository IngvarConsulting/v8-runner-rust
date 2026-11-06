---
id: INV.PLATFORM.MANAGED-POSTGRES-STOPS-THROUGH-PG-CTL
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/536
---

# PostgreSQL останавливает `pg_ctl`

Свой PostgreSQL останавливается `pg_ctl stop -m fast`, потому что так он успевает закрыть данные.
