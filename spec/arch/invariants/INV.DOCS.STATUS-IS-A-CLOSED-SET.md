---
id: INV.DOCS.STATUS-IS-A-CLOSED-SET
status: active
governs: process
decision: DEC.2026-09-16.STATUS-IS-A-CLOSED-SET
check: tests/arch_registry.rs::status_reads_active_planned_or_superseded
scope: [docs]
---

# `status` читается как `active`, `planned` или `superseded`

Поле `status` любой записи любого из трёх реестров принимает одно из трёх значений и
никакое другое. Запись с четвёртым значением реестром отклоняется: по `status` реестр
ветвит собственные проверки, и незнакомое слово там не нарушает правило, а отменяет
его.
