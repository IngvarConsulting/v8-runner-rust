---
id: INV.CONFIG.A-MANAGED-CLUSTER-DECLARES-NO-CONNECTION
check:
  - src/config/validate.rs::a_managed_cluster_declares_no_connection
  - tests/cli_cluster_managed.rs::a_declared_connection_next_to_the_managed_cluster_is_refused
  - src/config/validate.rs::a_projected_connection_of_the_managed_cluster_is_not_a_declared_one
---

# У кластера под управлением раннера нет строки соединения

`cluster.managed` объявляет вид цели кластера. Рядом с ней `connection` — отказ валидации: адрес `Srvr=` раннер
выводит из своей записи под замком и отдаёт его тем же исполнителям, что у обычной цели. Выведенная строка помечена и
отказа не вызывает.
