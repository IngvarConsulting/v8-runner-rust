---
id: INV.USE-CASES.A-MANAGED-CREATION-REFUSES-AN-EXISTING-DATABASE
check:
  - src/use_cases/managed_stand/standalone.rs::creating_over_an_existing_database_is_refused
---

# Создание управляемой базы не берёт существующую БД

`infobase create` при `dbms.managed` сначала спрашивает у своего PostgreSQL, есть ли БД с именем `dbms.name`, и если есть,
отказывает до запуска платформы, потому что `CrSQLDB=Y` молча взял бы её. Отказ называет выход: другое имя `dbms.name`
или удаление этой БД средствами PostgreSQL с паролем из файла, путь к которому называет `dbms status`.
