---
id: INV.USE-CASES.A-MANAGED-CREATION-REFUSES-AN-EXISTING-DATABASE
check:
  - src/use_cases/managed_stand/standalone.rs::creating_over_an_existing_database_is_refused
  - src/use_cases/managed_stand/cluster.rs::infobase_create_refuses_an_existing_database_before_the_platform
  - src/use_cases/managed_stand/cluster.rs::infobase_create_over_a_base_the_cluster_knows_marks_it_created
  - src/use_cases/managed_stand/cluster.rs::a_managed_recreate_sets_the_database_aside_and_drops_only_it
---

# Создание управляемой базы не берёт существующую БД

`infobase create` при `dbms.managed` сначала спрашивает у своего PostgreSQL, есть ли БД с именем `dbms.name`, и если есть,
отказывает до запуска платформы, потому что `CrSQLDB=Y` молча взял бы её. Отказ называет выход: другое имя `dbms.name`
`infobase create --recreate`, который заменяет базу (`INV.USE-CASES.A-MANAGED-RECREATE-DROPS-ONLY-ITS-OWN-DATABASE`),
или удаление этой БД средствами PostgreSQL с паролем из файла, путь к которому называет `dbms status`. В кластере раннера
`infobase create` сначала спрашивает `rac`: базу, которую кластер уже знает, команда не создаёт и отмечает созданной.
