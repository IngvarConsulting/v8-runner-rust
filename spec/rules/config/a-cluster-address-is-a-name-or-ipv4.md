---
id: INV.CONFIG.A-CLUSTER-ADDRESS-IS-A-NAME-OR-IPV4
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/378
---

# Адрес кластера — имя или IPv4

`cluster.ras` и `cluster.agent.address` принимают имя или адрес IPv4 с необязательным
портом. Адрес IPv6, в скобках или без, валидация отвергает и называет ключ: `rac` такой адрес
не разбирает, а `ras` слушает только IPv4 ([замер](../../../references/1c/confirmed-runtime-measurements.md)). То же относится к адресу RAS,
который раннер выводит из строки соединения. Адреса шлюза автономного сервера и подключения
к агенту это правило не касается.
