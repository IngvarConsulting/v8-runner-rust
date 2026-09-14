---
id: INV.CLI.NESTED-ORCHESTRATION-DOES-NOT-RELOCK
status: planned
governs: product
decision: DEC.2026-04-20.A-COMMAND-OWNS-THE-WORKPATH-EXCLUSIVELY
check: null
scope: [cli, use-cases]
---

# Вложенные шаги не берут блокировку повторно

Вложенная оркестрация работает под внешней блокировкой через явные внутренние входы и второй раз её не захватывает.
