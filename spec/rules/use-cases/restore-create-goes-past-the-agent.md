---
id: INV.USE-CASES.RESTORE-CREATE-GOES-PAST-THE-AGENT
check:
  - tests/cli_infobase.rs::restore_creates_an_absent_infobase_through_designer
---

# `infobase restore --create` идёт мимо агента

Сессия агента открывается к существующей базе, а `--create` ждёт, что базы ещё нет. Поэтому
с `--create` агент из цепочки умолчаний попадает в пропущенные с причиной, и восстановление
берёт следующий готовый исполнитель.
