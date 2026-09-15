---
id: INV.CONFIG.WORKPATH-IS-ALWAYS-LOCAL
status: active
governs: product
decision: DEC.2026-09-14.THE-TARGET-MAY-LIVE-ON-ANOTHER-MACHINE
check: tests/cli_agent_standalone.rs::a_work_path_on_the_target_side_is_refused
scope: [config]
---

# Рабочий каталог всегда локален

`workPath` указывает на файловую систему машины раннера. Путь на стороне цели рабочим каталогом не назначается, даже если он выглядит достижимым.
