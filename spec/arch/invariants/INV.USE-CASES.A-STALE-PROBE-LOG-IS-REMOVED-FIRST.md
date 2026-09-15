---
id: INV.USE-CASES.A-STALE-PROBE-LOG-IS-REMOVED-FIRST
status: active
governs: product
decision: DEC.2026-09-02.LOAD-COMPATIBILITY-STATES-ARE-CLOSED-AND-FAIL-CLOSED
check: src/platform/designer.rs::refuses_to_run_when_previous_platform_log_cannot_be_removed
scope: [use-cases]
---

# Прежний журнал пробы удаляется до запуска

Перед пробой совместимости прежний файл журнала удаляется, а ошибка удаления прекращает операцию: иначе решение принималось бы по чужому запуску.
