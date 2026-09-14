---
id: INV.USE-CASES.SOURCE-SET-SELECTION-IS-HONORED
status: active
governs: product
decision: DEC.2026-04-20.SOURCE-SET-IS-THE-UNIT-OF-ORCHESTRATION
check: tests/cli_build.rs::build_source_set_json_limits_steps_to_requested_source_set
scope: [use-cases]
---

# Выбор набора ограничивает шаги именно им

Названный набор ограничивает работу и состав шагов в ответе: чужие наборы не обрабатываются и в квитанции не появляются.
