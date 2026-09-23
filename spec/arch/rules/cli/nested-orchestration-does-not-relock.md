---
id: INV.CLI.NESTED-ORCHESTRATION-DOES-NOT-RELOCK
check: [tests/architecture_guardrails.rs::nested_orchestration_never_acquires_the_workspace_lock_inside_use_cases]
---

# Вложенные шаги не берут блокировку повторно

Вложенная оркестрация работает под внешней блокировкой через явные внутренние входы и второй раз её не захватывает.
