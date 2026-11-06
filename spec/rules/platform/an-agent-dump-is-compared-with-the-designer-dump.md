---
id: INV.PLATFORM.AN-AGENT-DUMP-IS-COMPARED-WITH-THE-DESIGNER-DUMP
check:
  - src/platform/designer.rs::main_configuration_exports_use_the_measured_designer_commands
  - tests/cli_agent_standalone.rs::a_full_dump_travels_through_sftp
  - tests/cli_agent_standalone.rs::a_download_through_the_gate_exports_the_main_configuration
---

# Выгрузка агента сравнена с выгрузкой Конфигуратора побайтно

Выгрузку одной и той же базы агентом и пакетным Конфигуратором сравнили побайтно на живой
платформе: файлы основной конфигурации (`config dump-config-to-files` и
`/DumpConfigToFiles`) и пакет конфигурации (`config dump-cfg` и `/DumpCfg`). Замер с датой,
сборкой, командами и перечнем расхождений записан
[в записи замеров](../../../references/1c/confirmed-runtime-measurements.md#выгрузка-агента-против-выгрузки-конфигуратора).
Проверки удерживают замеренную форму команд Конфигуратора и агента. Найденные расхождения названы в
описании исполнителя `agent` в `docs/CAPABILITIES.md`.
