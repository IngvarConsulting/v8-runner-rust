---
id: INV.DOCS.A-SUPERSESSION-IS-RECORDED-BY-BOTH-DECISIONS
status: active
governs: process
decision: DEC.2026-09-16.A-SUPERSESSION-IS-RECORDED-BY-BOTH-DECISIONS
check: [tests/arch_registry.rs::a_supersession_is_recorded_by_both_decisions, tests/arch_registry.rs::registry_records_match_the_published_schema_and_the_index_is_current]
scope: [docs]
---

# Обе половины замены названы и ведут к решениям

Символ в `superseded-by` и каждый символ в `supersedes` ведут к записи вида «решение».
Решение, названное преемником, называет предшественника в `supersedes`; решение,
названное в `supersedes`, называет преемника в `superseded-by`.
