---
id: INV.USE-CASES.A-SERVICE-STATUS-OPENS-NO-BASE
check:
  - tests/cli_dbms.rs::status_needs_no_build_and_opens_no_base
  - src/use_cases/managed_stand/mod.rs::status_of_a_running_service_runs_no_utility
---

# `status` служб не открывает базу

`status` не запускает Конфигуратор, клиент и `pg_ctl`. Живость проверяет проба порта, у кластера — `rac cluster list`.
