---
id: INV.PLATFORM.THE-RUNNER-NEVER-NAMES-A-DUMP-FORMAT
status: active
governs: product
decision: DEC.2026-09-16.ONLY-THE-HIERARCHICAL-DUMP-FORMAT-IS-SUPPORTED
check: tests/architecture_guardrails.rs::the_runner_never_names_a_dump_format
scope: [platform]
---

# Раскладка выгрузки платформе не называется

Ни одна команда, которую раннер отправляет платформе, не несёт аргумент раскладки выгрузки: ни `-Format` в пакетном Конфигураторе, ни `--format=` в агентском shell. Раскладка одна — платформенное умолчание.
