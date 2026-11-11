---
id: INV.CLI.INFOBASE-CREATE-IN-A-CLUSTER-NAMES-THE-CLUSTER-ADMINISTRATOR
check:
  - tests/cli_init.rs::cluster_creation_without_diagnostics_suggests_credentials_conditionally
  - tests/cli_init.rs::cluster_creation_reports_the_masked_platform_log_without_guessing_the_cause
---

# Отказ создания базы в кластере передаёт диагностику платформы

При отказе `infobase create` передаёт доступную диагностику `CREATEINFOBASE` с маскированием
паролей. Код выхода одинаков для разных причин, а при отсутствии реквизитов администратора
платформа может сообщить только о запрете окон (`DisableStartupDialogs`). Если диагностики
нет и администратор кластера не объявлен, отказ даёт условную подсказку: в кластере с
администраторами нужны `infobase.cluster.user` и `infobase.cluster.password`.

Источник: [`cli.html#map`](../../../docs/site/cli.html#map), замер
[#181](../../../references/1c/confirmed-runtime-measurements.md).
