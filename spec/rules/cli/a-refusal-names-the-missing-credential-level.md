---
id: INV.CLI.A-REFUSAL-NAMES-THE-MISSING-CREDENTIAL-LEVEL
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/212
---

# Отказ называет недостающий уровень учётных данных

Уровней три: пользователь базы (`user` секции базы), администратор кластера
(`cluster.user`), администратор центрального сервера (`cluster.agent.user`). `sessions list`
и `sessions terminate` запрашивают уровень кластера, а `sessions deny` и `sessions allow` —
уровни кластера и базы. Отказ при отсутствии нужных реквизитов называет уровень и ключ
секции (разрыв #212). Для `infobase create` в кластере с администраторами нужны реквизиты
уровня кластера. Диагностику `CREATEINFOBASE` описывает правило
`INV.CLI.INFOBASE-CREATE-IN-A-CLUSTER-NAMES-THE-CLUSTER-ADMINISTRATOR`.

Источник: [`cli.html#map`](../../../docs/site/cli.html#map),
[`platform.html#t59`](../../../docs/site/platform.html#t59).
