---
id: INV.USE-CASES.A-SERVICE-STATUS-OPENS-NO-BASE
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/536
---

# `status` служб не открывает базу

`status` не запускает Конфигуратор, клиент и `pg_ctl`. Живость проверяет проба порта, у кластера — `rac cluster list`.
