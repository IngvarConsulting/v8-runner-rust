---
id: INV.USE-CASES.IBCMD-BUILDS-A-PACKAGE-IN-A-THROWAWAY-BASE
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/236
---

# `convert` в пакет через `ibcmd` идёт во временной базе раннера

Без существующей базы `ibcmd config import --out` не работает, а без `--out` та же
команда загружает исходники в базу ([замер](../../../references/1c/confirmed-runtime-measurements.md)). Поэтому `convert` в пакет,
выполняемый `ibcmd`, собирает его во временной базе раннера тем же владельцем, что `make`
(`INV.USE-CASES.MAKE-BUILDS-PACKAGES-FROM-SOURCES-IN-A-THROWAWAY-BASE`): база под
`workPath` со своим каталогом данных `--data`, импорт всегда с `--out`, база убирается после
шага. База проекта в этом не участвует, и для пользователя сборка остаётся сборкой без базы.
