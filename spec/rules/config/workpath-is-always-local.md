---
id: INV.CONFIG.WORKPATH-IS-ALWAYS-LOCAL
check:
  - tests/cli_agent_standalone.rs::a_work_path_on_the_target_side_is_refused
  - tests/cli_agent_standalone.rs::gate_commands_carry_target_side_relative_paths
  - tests/cli_standalone_direct_gate.rs::the_designer_by_the_direct_gate_keeps_the_files_on_the_runner_side
---

# Рабочий каталог всегда локален

`workPath` указывает на файловую систему машины раннера. Путь на стороне цели рабочим
каталогом не назначается, даже если он выглядит достижимым: `workPath`, пересекающийся с
каталогом обмена SSH-шлюза `standalone.exchange.dir`, — ошибка валидации.

Журнал платформы, который называет ответ команды на автономном сервере, — файл под `workPath`
на машине раннера: у Конфигуратора по прямому шлюзу это его журнал `/Out`, у агента по
SSH-шлюзу — журнал сессии. Журналов на стороне цели ответ не обещает.
