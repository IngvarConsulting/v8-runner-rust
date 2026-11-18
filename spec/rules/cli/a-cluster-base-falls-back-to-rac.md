---
id: INV.CLI.A-CLUSTER-BASE-FALLS-BACK-TO-RAC
check:
  - tests/cli_init.rs::a_missing_designer_uses_rac_to_create_an_unregistered_base
---

# Базу в кластере запасным путём создаёт `rac`

Когда Конфигуратор недоступен и создание в кластере не началось, `infobase create` создаёт базу через
`rac infobase create --create-database` с теми же реквизитами `dbms` и администратором
кластера. Ответ `infobase : <uuid>` значит «создана». Вызову нужны сервер администрирования — объявленный `cluster.ras` или `ras`, поднятый
раннером, — и идентификатор кластера из `rac cluster list`.

После неопределённого исхода `CREATEINFOBASE` повторного создания нет. Неудачный вызов
может оставить БД в СУБД до регистрации. Ответ называет необходимость проверки СУБД.

Источник: [`deployments.html`](../../../docs/site/deployments.html),
[`platform.html#t73`](../../../docs/site/platform.html#t73).
