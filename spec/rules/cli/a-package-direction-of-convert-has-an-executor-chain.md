---
id: INV.CLI.A-PACKAGE-DIRECTION-OF-CONVERT-HAS-AN-EXECUTOR-CHAIN
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/236
---

# Направление `convert` с пакетом выполняет цепочка исполнителей

Для направлений с пакетом есть цепочка исполнителей `ibcmd` → `ibcmd-rs`, и квитанция
называет выбранного; `ibcmd` работает во временной базе раннера
(`INV.USE-CASES.IBCMD-BUILDS-A-PACKAGE-IN-A-THROWAWAY-BASE`); EDT ↔ XML по-прежнему выполняет `1cedtcli`. Отсутствие инструментов
отвечает по `INV.WIRE.A-MISSING-TOOL-IS-AN-ENVIRONMENT-FAILURE`. Квитанция с исполнителем
меняет форму `CTR.WIRE.CONVERT-DATA`, и её версия растёт вместе с этой работой.
