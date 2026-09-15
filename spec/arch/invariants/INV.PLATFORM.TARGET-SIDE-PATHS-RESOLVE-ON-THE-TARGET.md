---
id: INV.PLATFORM.TARGET-SIDE-PATHS-RESOLVE-ON-THE-TARGET
status: active
governs: product
decision: DEC.2026-09-14.THE-TARGET-MAY-LIVE-ON-ANOTHER-MACHINE
check: tests/cli_agent_standalone.rs::gate_commands_carry_target_side_relative_paths
scope: [platform]
---

# Пути в командах цели разрешаются на её стороне

Каталог выгрузки и имя файла, переданные агенту или шлюзу, разрешаются на стороне цели. Раннер не складывает такой путь из своего рабочего каталога и не читает его напрямую.
