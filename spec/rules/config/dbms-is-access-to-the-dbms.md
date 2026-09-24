---
id: INV.CONFIG.DBMS-IS-ACCESS-TO-THE-DBMS
check:
  - src/config/validate.rs::a_server_connection_without_dbms_is_valid_for_every_provider
  - tests/cli_infobase.rs::a_server_connection_without_dbms_still_exports_through_the_designer
---

# Секция `dbms` — это доступ к СУБД, а не признак серверной базы

Секция `infobase.dbms` требуется только там, где в СУБД идёт сам раннер: создать серверную
информационную базу и любая операция `ibcmd` с серверной базой — `ibcmd` работает с её
данными напрямую.

Проверку конфигурации серверная база без секции проходит. Исполнитель, который в СУБД не
ходит, работает с уже существующей серверной базой — сборка, выгрузка, экспорт
конфигурации — без неё. Требовать полный контракт СУБД у операции, которая в СУБД не ходит,
значит просить лишний секрет.
