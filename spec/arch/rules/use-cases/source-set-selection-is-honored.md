---
id: INV.USE-CASES.SOURCE-SET-SELECTION-IS-HONORED
check: [tests/cli_build.rs::build_source_set_json_limits_steps_to_requested_source_set]
---

# Выбор набора ограничивает шаги именно им

Названный набор ограничивает работу и состав шагов в ответе: чужие наборы не обрабатываются и в квитанции не появляются.
