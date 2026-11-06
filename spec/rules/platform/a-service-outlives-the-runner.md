---
id: INV.PLATFORM.A-SERVICE-OUTLIVES-THE-RUNNER
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/536
---

# Служба переживает выход раннера

Служба запускается вне Job раннера, со своей скрытой консолью и без наследованных дескрипторов родителя, вывод
идёт в журнал под `workPath`. Если раннер сам работает в Job с kill-on-close, служба отделяется
`CREATE_BREAKAWAY_FROM_JOB`. Job этого не разрешает — отказ `environment_unavailable` до запуска службы.
