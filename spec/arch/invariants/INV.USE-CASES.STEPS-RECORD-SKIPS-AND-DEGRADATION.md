---
id: INV.USE-CASES.STEPS-RECORD-SKIPS-AND-DEGRADATION
status: active
governs: product
decision: DEC.2026-04-21.EXECUTION-OUTCOME-IS-THE-CANONICAL-RESULT
check: tests/cli_dump.rs::dump_text_warning_shows_degraded_fallback_reason
scope: [use-cases]
---

# Пропуск и деградация попадают в шаги, а не теряются

Пропущенный шаг и шаг, выполненный в более полном режиме, остаются в ответе с причиной: молчание неотличимо от успеха.
