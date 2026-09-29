---
id: INV.CLI.A-COMMAND-WITHOUT-ORIGIN-NAMES-THE-MISSING-STEP
check: [tests/cli_infobases.rs::a_command_without_origin_names_the_missing_step]
---

# Команда без объявленной базы называет шаг

Команда, которой нужна база, при отсутствующем `origin` и без `--infobase` отказывает до
запуска платформы и называет ключ `--infobase <имя|строка соединения>`; выбирать базу
наугад она не вправе. Какие выходы называет этот отказ, говорит
`INV.CLI.A-REFUSAL-WITHOUT-AN-OWN-BASE-NAMES-THE-WAYS-OUT`.
