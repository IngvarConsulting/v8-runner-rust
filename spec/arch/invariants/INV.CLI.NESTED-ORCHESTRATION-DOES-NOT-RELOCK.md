---
id: INV.CLI.NESTED-ORCHESTRATION-DOES-NOT-RELOCK
status: active
governs: product
decision: DEC.2026-04-20.A-COMMAND-OWNS-THE-WORKPATH-EXCLUSIVELY
check: tests/architecture_guardrails.rs::nested_orchestration_never_acquires_the_workspace_lock_inside_use_cases
scope: [cli, use-cases]
---

# Вложенные шаги не берут блокировку повторно

Вложенная оркестрация работает под внешней блокировкой через явные внутренние входы и второй раз её не захватывает.
