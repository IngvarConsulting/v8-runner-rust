---
id: INV.CONFIG.A-CLUSTER-ADDRESS-IS-A-NAME-OR-IPV4
check:
  - src/config/validate.rs::a_cluster_address_is_a_host_with_an_optional_port
  - src/config/validate.rs::an_ipv6_cluster_address_is_refused_naming_the_key_and_the_reason
  - tests/cli_cluster_section.rs::a_malformed_ras_address_is_refused_naming_the_key
---

# Адрес кластера — имя или IPv4

`cluster.ras` и `cluster.agent.address` принимают имя или адрес IPv4 с необязательным
портом. Адрес IPv6, в скобках или без, валидация отвергает и называет ключ: `rac` такой адрес
не разбирает, а `ras` слушает только IPv4 ([замер](../../../references/1c/confirmed-runtime-measurements.md)). Адрес, который раннер выводит
из строки соединения, валидация не проверяет — его держит
`INV.CONFIG.A-DERIVED-CLUSTER-ADDRESS-IS-A-NAME-OR-IPV4`. Адреса шлюза автономного сервера
и подключения к агенту это правило не касается.
