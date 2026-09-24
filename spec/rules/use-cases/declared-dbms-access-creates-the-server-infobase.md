---
id: INV.USE-CASES.DECLARED-DBMS-ACCESS-CREATES-THE-SERVER-INFOBASE
check:
  - tests/cli_init.rs::init_ibcmd_server_provisions_infobase_without_precheck
  - tests/cli_init.rs::init_designer_non_zero_create_exit_stays_fatal_even_when_marker_appears
---

# Объявленный доступ к СУБД создаёт серверную базу

При полном контракте `infobase.dbms` команда создания обеспечивает наличие серверной
информационной базы и не пропускает шаг молча; отдельного ключа для этого нет.

Итог шага определяется по наблюдаемому состоянию базы, а не по тексту сообщения утилиты:
ненулевой код выхода остаётся фатальным, даже если маркер появился.
