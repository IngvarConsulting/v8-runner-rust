---
id: INV.DOCS.A-SUPERSEDED-DECISION-NAMES-ITS-SUCCESSOR
status: active
governs: process
decision: DEC.2026-09-16.A-SUPERSEDED-DECISION-NAMES-ITS-SUCCESSOR
check: [tests/arch_registry.rs::a_superseded_decision_names_its_successor, tests/arch_registry.rs::registry_records_match_the_published_schema_and_the_index_is_current]
scope: [docs]
---

# Статус замены и имя преемника стоят вместе

У решения со `status: superseded` поле `superseded-by` называет символ, а не `null`.
У решения с любым другим статусом `superseded-by` пуст.
