---
id: INV.PLATFORM.AN-AGENT-DUMP-IS-COMPARED-WITH-THE-DESIGNER-DUMP
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/230
---

# Выгрузка агента сравнена с выгрузкой Конфигуратора побайтно

Выгрузку одной и той же базы агентом и пакетным Конфигуратором сравнили побайтно на живой
платформе: файлы основной конфигурации (`config dump-config-to-files` и
`/DumpConfigToFiles`) и пакет конфигурации (`config dump-cfg` и `/DumpCfg`). Замер с датой,
сборкой, командами и перечнем расхождений записан в
`references/1c/confirmed-runtime-measurements.md`, а найденные расхождения названы в
описании исполнителя `agent` в `docs/CAPABILITIES.md`.
