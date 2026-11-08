---
id: INV.CONFIG.A-MANAGED-CLUSTER-DECLARES-NO-CONNECTION
check:
  - src/config/validate.rs::a_managed_cluster_declares_no_connection
---

# У кластера под управлением раннера нет строки соединения

`cluster.managed` объявляет вид цели кластера. Рядом с ней `connection` — отказ валидации: адрес `Srvr=` раннер
выводит из своей записи под замком и отдаёт его тем же исполнителям, что у обычной цели.
