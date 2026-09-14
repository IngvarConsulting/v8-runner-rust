---
id: INV.USE-CASES.COMPATIBILITY-MATRIX-IS-EXHAUSTIVE
status: active
governs: product
decision: DEC.2026-09-02.LOAD-COMPATIBILITY-STATES-ARE-CLOSED-AND-FAIL-CLOSED
check: src/use_cases/load_artifact.rs::the_compatibility_matrix_answers_every_combination_and_never_permits_an_unproven_one
scope: [use-cases]
---

# Матрица режимов и состояний перечислена исчерпывающе

Сочетание режима и состояния совместимости разбирается без ветки по умолчанию: новое состояние обязано быть названо явно, иначе код не собирается.
