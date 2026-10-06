---
id: INV.USE-CASES.IBCMD-BUILDS-A-PACKAGE-IN-A-THROWAWAY-BASE
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/207
---

# `ibcmd` собирает пакет во временной базе раннера

Без существующей базы `ibcmd config import --out` не работает, а без `--out` та же
команда загружает исходники в базу ([замер](../../../references/1c/confirmed-runtime-measurements.md)). Поэтому `make` и `convert` в пакет,
выполняемые `ibcmd`, создают временную файловую базу раннера под `workPath` со своим
каталогом данных `--data`, вызывают импорт всегда с `--out` и убирают базу после шага.
База проекта в этом не участвует, и для пользователя сборка остаётся сборкой без базы.
Осиротевшую временную базу уборка узнаёт как свою по
`INV.USE-CASES.CLEANUP-TOUCHES-ONLY-ITS-OWN-ARTEFACTS`.
