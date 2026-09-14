---
id: INV.USE-CASES.A-STALE-PROBE-LOG-IS-REMOVED-FIRST
status: planned
governs: product
decision: DEC.2026-09-02.LOAD-COMPATIBILITY-STATES-ARE-CLOSED-AND-FAIL-CLOSED
check: null
scope: [use-cases]
---

# Прежний журнал пробы удаляется до запуска

Перед пробой совместимости прежний файл журнала удаляется, а ошибка удаления прекращает операцию: иначе решение принималось бы по чужому запуску.
