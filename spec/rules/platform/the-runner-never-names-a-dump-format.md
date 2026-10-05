---
id: INV.PLATFORM.THE-RUNNER-NEVER-NAMES-A-DUMP-FORMAT
check: [tests/architecture_guardrails.rs::the_runner_never_names_a_dump_format, tests/architecture_guardrails.rs::the_dump_format_guard_sees_every_spelling]
---

# Раскладка выгрузки платформе не называется

Ни одна команда, которую раннер отправляет платформе, не несёт аргумент раскладки выгрузки
в файлы: ни `-Format` в любом регистре у Конфигуратора, ни `--format` у агента. Это касается
выгрузки и загрузки конфигурации, расширений и внешних обработок.

Раскладка одна — иерархическая, она же платформенное умолчание (решение владельца от
16.09.2026). `--output-format` агентской сессии задаёт форму ответа, а не раскладку.
