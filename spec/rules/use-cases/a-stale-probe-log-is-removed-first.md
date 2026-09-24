---
id: INV.USE-CASES.A-STALE-PROBE-LOG-IS-REMOVED-FIRST
check: [src/platform/designer.rs::refuses_to_run_when_previous_platform_log_cannot_be_removed]
---

# Прежний журнал пробы удаляется до запуска

Перед пробой совместимости прежний файл журнала удаляется, а ошибка удаления прекращает операцию: иначе решение принималось бы по чужому запуску.
