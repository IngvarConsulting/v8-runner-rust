---
id: INV.CLI.EXIT-CODE-REFLECTS-THE-FAILURE-KIND
status: planned
governs: product
decision: DEC.2026-04-20.BUSINESS-FAILURES-ARE-NOT-TRANSPORT-FAULTS
check: null
scope: [cli]
---

# Код выхода различает отказ сценария и сбой обвязки

Ожидаемый отказ сценария и сбой обвязки дают разные коды выхода: вызывающий отличает «не получилось» от «сломалось» без чтения текста.
