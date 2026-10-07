---
id: INV.CLI.INFOBASE-CREATE-IN-A-CLUSTER-NAMES-THE-CLUSTER-ADMINISTRATOR
check:
  - tests/cli_init.rs::a_cluster_with_administrators_refuses_naming_the_cluster_administrator_level
---

# Отказ создания базы в кластере без администратора называет его уровень

Если `infobase create` в кластере не удался, а администратор кластера не объявлен, отказ
называет уровень администратора кластера и ключи `infobase.cluster.user` и
`infobase.cluster.password`. Причину отказа по прозе платформы раннер не угадывает: уровень
назван, потому что в кластере с заполненным списком администраторов без него создание не
проходит. Это часть правила `INV.CLI.A-REFUSAL-NAMES-THE-MISSING-CREDENTIAL-LEVEL` для
`infobase create`.

Источник: [`cli.html#map`](../../../docs/site/cli.html#map), замер
[#181](../../../references/1c/confirmed-runtime-measurements.md).
