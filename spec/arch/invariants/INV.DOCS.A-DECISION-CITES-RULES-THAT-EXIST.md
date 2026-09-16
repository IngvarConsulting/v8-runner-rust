---
id: INV.DOCS.A-DECISION-CITES-RULES-THAT-EXIST
status: active
governs: process
decision: DEC.2026-09-16.A-DECISION-CITES-RULES-THAT-EXIST
check: [tests/arch_registry.rs::a_decision_cites_rules_that_exist, tests/arch_registry.rs::registry_records_match_the_published_schema_and_the_index_is_current]
scope: [docs]
---

# Правило, названное решением, есть в реестре

Каждый символ в `establishes` и в `changes` принадлежит записи реестра вида «инвариант»
или «контракт». Символ без записи и символ решения отклоняются.
