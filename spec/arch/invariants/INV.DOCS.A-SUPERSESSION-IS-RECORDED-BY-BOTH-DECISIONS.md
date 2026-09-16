---
id: INV.DOCS.A-SUPERSESSION-IS-RECORDED-BY-BOTH-DECISIONS
status: active
governs: process
decision: DEC.2026-09-16.A-SUPERSESSION-IS-RECORDED-BY-BOTH-DECISIONS
check: [tests/arch_registry.rs::a_supersession_is_recorded_by_both_decisions, tests/arch_registry.rs::registry_records_match_the_published_schema_and_the_index_is_current]
scope: [docs]
---

# Обе половины замены отвечают друг другу

Решение, названное в `superseded-by` другого решения, называет его в `supersedes`;
решение, названное в `supersedes`, называет назвавшего в `superseded-by`. Сверяются
только пары, оба конца которых уже разрешились в решения.
