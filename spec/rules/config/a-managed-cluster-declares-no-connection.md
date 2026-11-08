---
id: INV.CONFIG.A-MANAGED-CLUSTER-DECLARES-NO-CONNECTION
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/538
---

# У кластера под управлением раннера нет строки соединения

`cluster.managed` объявляет вид цели кластера. Рядом с ней `connection` — отказ валидации: адрес `Srvr=` раннер
выводит из своей записи под замком и отдаёт его тем же исполнителям, что у обычной цели.
