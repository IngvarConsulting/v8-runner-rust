---
id: INV.DOCS.GOVERNS-IS-A-CLOSED-AXIS
status: active
governs: process
decision: DEC.2026-09-16.GOVERNS-IS-A-CLOSED-AXIS
check: tests/arch_registry.rs::governs_reads_product_or_process
scope: [docs]
---

# `governs` читается как `product` или `process`

Поле `governs` любой записи любого из трёх реестров принимает одно из двух значений
и никакое другое. Запись с третьим значением реестром отклоняется.
