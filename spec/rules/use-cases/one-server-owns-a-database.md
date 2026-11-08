---
id: INV.USE-CASES.ONE-SERVER-OWNS-A-DATABASE
check:
  - src/use_cases/managed_stand/cluster.rs::a_running_cluster_refuses_ibcmd_over_the_same_database
  - src/use_cases/managed_stand/cluster.rs::a_running_standalone_of_the_same_base_refuses_the_cluster
  - src/use_cases/managed_stand/cluster.rs::a_running_cluster_refuses_the_standalone_start_over_the_same_database
  - src/use_cases/managed_stand/cluster.rs::the_cluster_of_another_base_over_the_same_database_owns_it
  - src/use_cases/managed_stand/cluster.rs::localhost_and_the_loopback_address_are_different_databases
---

# Одну БД держит один сервер

Пока свой кластер этой базы работает, `standalone start` над СУБД и `infobase create` автономного сервера
(`ibcmd --create-database`) отказывают до запуска платформы и называют `cluster stop`. Тот же отказ получает другая
объявленная база, чей `dbms.server` и `dbms.name` совпадают с базой работающего своего кластера, — и `infobase create`
в её собственном кластере тоже. Адрес сравнивается
без учёта регистра и лишних пробелов, иначе буквально: `localhost` и `127.0.0.1` — разные адреса. Обратно: `cluster
start` при работающем своём `ibsrv` этой базы отказывает и называет `standalone stop`.
