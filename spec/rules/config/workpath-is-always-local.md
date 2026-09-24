---
id: INV.CONFIG.WORKPATH-IS-ALWAYS-LOCAL
check: [tests/cli_agent_standalone.rs::a_work_path_on_the_target_side_is_refused]
---

# Рабочий каталог всегда локален

`workPath` указывает на файловую систему машины раннера. Путь на стороне цели рабочим каталогом не назначается, даже если он выглядит достижимым.
