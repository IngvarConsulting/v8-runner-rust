---
id: INV.CLI.A-REFUSAL-NAMES-THE-MISSING-CREDENTIAL-LEVEL
check:
  - tests/cli_sessions.rs::lock_commands_name_the_missing_credential_level
  - tests/cli_init.rs::cluster_creation_without_diagnostics_suggests_credentials_conditionally
---

# Отказ называет недостающий уровень учётных данных

Уровней три: пользователь базы (`user` секции базы), администратор кластера
(`cluster.user`), администратор центрального сервера (`cluster.agent.user`). `sessions list`
и `sessions terminate` запрашивают уровень кластера, а `sessions deny` и `sessions allow` —
уровни кластера и базы. Отказ при отсутствии нужных реквизитов называет уровень и ключ
секции. Просмотр и завершение допускают отсутствие реквизитов на кластере без администраторов.
Для `deny` и `allow` нужны реквизиты кластера и базы и версия `rac`, принимающая ключи пользователя ИБ.
На автономном сервере нужен только уровень базы. Для `infobase create` в кластере с администраторами нужны реквизиты
уровня кластера. Диагностику `CREATEINFOBASE` описывает правило
`INV.CLI.INFOBASE-CREATE-IN-A-CLUSTER-NAMES-THE-CLUSTER-ADMINISTRATOR`.

Источник: [`cli.html#map`](../../../docs/site/cli.html#map),
[`platform.html#t59`](../../../docs/site/platform.html#t59).
