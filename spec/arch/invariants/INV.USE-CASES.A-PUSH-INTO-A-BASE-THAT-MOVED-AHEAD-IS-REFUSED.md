---
id: INV.USE-CASES.A-PUSH-INTO-A-BASE-THAT-MOVED-AHEAD-IS-REFUSED
status: planned
governs: product
decision: DEC.2026-09-21.THE-GENERATION-GUARDS-EVERY-EXCHANGE
check: null
scope: [use-cases, wire]
---

# Отправка в ушедшую вперёд базу отклоняется

`push` без `--force`, увидев поколение базы, отличное от записанного в памяти,
отказывает родом `non_fast_forward` до загрузки и называет оба поколения и следующий
шаг.
