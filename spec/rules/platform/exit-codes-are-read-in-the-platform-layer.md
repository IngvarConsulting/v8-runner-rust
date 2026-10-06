---
id: INV.PLATFORM.EXIT-CODES-ARE-READ-IN-THE-PLATFORM-LAYER
check:
  - tests/architecture_guardrails.rs::scenarios_receive_an_outcome_not_an_exit_code
  - tests/architecture_guardrails.rs::the_exit_code_finder_sees_every_reading_in_a_scenario
---

# Код выхода утилиты читает слой платформы

Что значит код выхода утилиты платформы, решает слой `platform`: это знание об инструменте —
у `/CompareCfg` ноль значит «сравнение состоялось», — и живёт оно рядом с адаптером.
Сценарий получает исход, а не код: `ProcessResult::outcome` отдаёт удачу или отказ с его
кодом, а вердикт проверки Конфигуратора отдаёт `designer::syntax_check_status`. Код в
тексте ответа сценарий называет, но с числом его не сравнивает.
