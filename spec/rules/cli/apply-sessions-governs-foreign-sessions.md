---
id: INV.CLI.APPLY-SESSIONS-GOVERNS-FOREIGN-SESSIONS
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/211
---

# `apply --sessions` решает судьбу чужих сеансов

`apply --sessions disable|force` управляет чужими сеансами, когда применению нужен
монопольный доступ; умолчание — `disable`, и чужие сеансы тогда не трогаются.

`force` сначала идёт ключом самого исполнителя: `-SessionTerminate` у Конфигуратора,
`--session-terminate` у `ibcmd config apply` и у агента в `config update-db-cfg`. Если
исполнитель завершить сеансы не может, их завершает средство администрирования вида базы:
`rac` у кластера, `ibcmd session` у автономного сервера. Если не может и оно (у файловой
базы его нет, или не хватает учётных данных), применение не начинается, а отказ называет
причину; недостающий уровень учётных данных называется по
`INV.CLI.A-REFUSAL-NAMES-THE-MISSING-CREDENTIAL-LEVEL`.
