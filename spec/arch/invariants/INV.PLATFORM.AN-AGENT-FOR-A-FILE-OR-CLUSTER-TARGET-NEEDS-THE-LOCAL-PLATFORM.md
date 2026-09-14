---
id: INV.PLATFORM.AN-AGENT-FOR-A-FILE-OR-CLUSTER-TARGET-NEEDS-THE-LOCAL-PLATFORM
status: planned
governs: product
decision: DEC.2026-09-14.ONLY-A-STANDALONE-SERVER-ANSWERS-WITHOUT-BEING-STARTED
check: null
scope: [platform, config]
---

# Агент для файловой и кластерной цели требует локальной платформы

Выбор провайдера `agent` для цели `File=` или `Srvr=…;Ref=…` без найденной локальной
платформы — отказ, а не попытка подключения. Исключение одно: конфиг явно назвал уже
поднятую точку входа (`attach`).
