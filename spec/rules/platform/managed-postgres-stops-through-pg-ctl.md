---
id: INV.PLATFORM.MANAGED-POSTGRES-STOPS-THROUGH-PG-CTL
check:
  - src/use_cases/managed_stand/mod.rs::stop_goes_through_pg_ctl_in_fast_mode
---

# PostgreSQL останавливает `pg_ctl`

Свой PostgreSQL останавливается `pg_ctl stop -m fast`, потому что так он успевает закрыть данные.
