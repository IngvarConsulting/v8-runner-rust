---
id: INV.CLI.A-COMMAND-WITHOUT-ORIGIN-NAMES-THE-MISSING-STEP
status: planned
governs: product
decision: DEC.2026-09-21.INFOBASES-ARE-A-NAMED-MAP-WITH-ORIGIN-AS-THE-DEFAULT
check: null
scope: [cli, config]
---

# Команда без объявленной базы называет шаг

Команда, которой нужна база, при отсутствующем `origin` и без `--infobase` отказывает до
запуска платформы и называет `init --infobase …` или `--infobase <имя>`; выбирать базу
наугад она не вправе.
