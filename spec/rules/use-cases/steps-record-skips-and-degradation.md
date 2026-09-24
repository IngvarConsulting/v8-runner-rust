---
id: INV.USE-CASES.STEPS-RECORD-SKIPS-AND-DEGRADATION
check: [tests/cli_dump.rs::dump_text_warning_shows_degraded_fallback_reason]
---

# Пропуск и деградация попадают в шаги, а не теряются

Пропущенный шаг и шаг, выполненный в более полном режиме, остаются в ответе с причиной: молчание неотличимо от успеха.
