---
id: INV.CLI.A-COMMAND-WITHOUT-ORIGIN-NAMES-THE-MISSING-STEP
status: active
governs: product
decision: DEC.2026-09-21.INFOBASES-ARE-A-NAMED-MAP-WITH-ORIGIN-AS-THE-DEFAULT
check: tests/cli_infobases.rs::a_command_without_origin_names_the_missing_step
scope: [cli, config]
---

# Команда без объявленной базы называет шаг

Команда, которой нужна база, при отсутствующем `origin` и без `--infobase` отказывает до
запуска платформы и называет шаг: ключ `--infobase <имя|строка соединения>` либо
объявление `infobases.origin.connection` в местном слое; выбирать базу наугад она не
вправе.
