---
id: INV.CLI.A-POSITIONAL-ARGUMENT-NEVER-NAMES-A-BASE
status: planned
governs: product
decision: DEC.2026-09-21.COMMANDS-FOLLOW-THE-GIT-VOCABULARY
check: null
scope: [cli]
---

# Позиционный аргумент никогда не называет базу

Позиционный аргумент ни одной команды не разбирается как имя базы или строка соединения;
у команд, принимающих набор исходников, позиционный — набор, и чужое значение на этом
месте — отказ, а не догадка. Базу называет ключ `--infobase`.
