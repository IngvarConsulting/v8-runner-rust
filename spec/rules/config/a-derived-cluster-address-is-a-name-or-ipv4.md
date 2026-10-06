---
id: INV.CONFIG.A-DERIVED-CLUSTER-ADDRESS-IS-A-NAME-OR-IPV4
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/213
---

# Адрес кластера, выведенный из строки соединения, — имя или IPv4

Без `cluster.ras` адрес администрирования раннер выводит по порядку:
`cluster.agent.address`, иначе первый непустой сервер `Srvr=` (или `/S`) с портом агента по
умолчанию. Хост IPv6, взятый из строки соединения, — отказ операции, которой адрес нужен, с советом объявить `cluster.ras` или
`cluster.agent.address` именем или IPv4: `rac` такой адрес не разбирает, а `ras` слушает
только IPv4 ([замер](../../../references/1c/confirmed-runtime-measurements.md)). Объявленный
`cluster.agent.address` строку не читает, и IPv6 в `Srvr=` рядом с ним отказа не даёт.
