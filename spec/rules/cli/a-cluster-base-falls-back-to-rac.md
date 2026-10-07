---
id: INV.CLI.A-CLUSTER-BASE-FALLS-BACK-TO-RAC
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/180
---

# Базу в кластере запасным путём создаёт `rac`

Когда Конфигуратор базу в кластере не создал, `infobase create` создаёт её через
`rac infobase create --create-database` с теми же реквизитами `dbms` и администратором
кластера. Ответ `infobase : <uuid>` значит «создана». Вызову нужен идентификатор кластера, а
вывод `rac cluster list`, из которого его берут, не записан ни одним замером.

Источник: [`deployments.html`](../../../docs/site/deployments.html),
[`platform.html#t73`](../../../docs/site/platform.html#t73).
