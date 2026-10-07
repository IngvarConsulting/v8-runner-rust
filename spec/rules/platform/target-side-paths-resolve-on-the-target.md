---
id: INV.PLATFORM.TARGET-SIDE-PATHS-RESOLVE-ON-THE-TARGET
check:
  - tests/cli_agent_standalone.rs::gate_commands_carry_target_side_relative_paths
  - tests/cli_standalone_direct_gate.rs::the_designer_by_the_direct_gate_keeps_the_files_on_the_runner_side
---

# Пути в командах цели разрешаются на её стороне

Каталог выгрузки и имя файла, переданные агенту через SSH-шлюз автономного сервера,
разрешаются на стороне цели — относительно каталога пользователя шлюза. Раннер не складывает
такой путь из своего рабочего каталога и не читает его напрямую.

Конфигуратор по прямому шлюзу работает на машине раннера, и пути в его командной строке —
пути этой машины: каталог исходников проекта, каталог выгрузки рядом с ним, журнал `/Out` под
`workPath`. Путей стороны цели он не получает.
