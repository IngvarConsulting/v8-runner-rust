---
id: INV.CLI.A-REFUSAL-NAMES-THE-MISSING-CREDENTIAL-LEVEL
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/233
---

# Отказ называет недостающий уровень учётных данных

Уровней три: пользователь базы (`user` секции базы), администратор кластера
(`cluster.user`), администратор центрального сервера (`cluster.agent.user`). Операция
запрашивает только нужный ей уровень — `sessions list` и `sessions terminate` кластер,
`sessions deny` и `sessions allow` кластер и базу, `infobase create` в кластере с
заполненным списком администраторов кластер, — и отказ без него называет уровень и
ключ секции, а не «неверный пароль». Проверки допишут `sessions` (#212) и
`infobase create` (#204).

Источник: [`cli.html#map`](../../../docs/site/cli.html#map),
[`platform.html#t59`](../../../docs/site/platform.html#t59).
