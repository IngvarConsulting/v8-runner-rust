---
id: INV.PLATFORM.AN-AGENT-FOR-A-FILE-OR-CLUSTER-TARGET-NEEDS-THE-LOCAL-PLATFORM
check: [tests/cli_dump_agent.rs::a_managed_agent_without_the_local_platform_is_refused_before_any_connection]
---

# Агент для файловой и кластерной цели требует локальной платформы

Выбор провайдера `agent` для цели `File=` или `Srvr=…;Ref=…` без найденной локальной
платформы — отказ, а не попытка подключения. Исключение одно: конфиг явно назвал уже
поднятую точку входа (`attach`).
