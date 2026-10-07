---
id: INV.CLI.A-CLUSTER-BASE-FALLS-BACK-TO-RAC
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/213
---

# Базу в кластере запасным путём создаёт `rac`

Когда Конфигуратор базу в кластере не создал, `infobase create` создаёт её через
`rac infobase create --create-database` с теми же реквизитами `dbms` и администратором
кластера. Ответ `infobase : <uuid>` значит «создана». Вызову нужны сервер администрирования — объявленный `cluster.ras` или `ras`, поднятый
раннером, — и идентификатор кластера из `rac cluster list`.

Источник: [`deployments.html`](../../../docs/site/deployments.html),
[`platform.html#t73`](../../../docs/site/platform.html#t73).
