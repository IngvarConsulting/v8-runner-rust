---
id: INV.DOCS.A-SYMBOL-RESOLVES-TO-A-RECORD
status: active
governs: process
decision: DEC.2026-09-16.A-SYMBOL-RESOLVES-TO-A-RECORD
check: tests/arch_registry.rs::every_symbol_a_decision_names_resolves_to_a_record
scope: [docs]
---

# Символ, названный решением, разрешается в запись нужного вида

Символы из `establishes` разрешаются в контракт или инвариант, символы из `supersedes` и `superseded-by` — в решение. Ненайденный символ и символ чужого вида реестр отклоняет, называя запись, которой не хватает. `establishes` заменённого решения — история, и он не разрешается.
