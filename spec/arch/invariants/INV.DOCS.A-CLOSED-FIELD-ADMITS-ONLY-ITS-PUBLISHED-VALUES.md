---
id: INV.DOCS.A-CLOSED-FIELD-ADMITS-ONLY-ITS-PUBLISHED-VALUES
status: active
governs: process
decision: DEC.2026-09-16.A-CLOSED-FIELD-ADMITS-ONLY-ITS-PUBLISHED-VALUES
check: [tests/arch_registry.rs::a_closed_field_admits_only_its_published_values, tests/arch_registry.rs::registry_records_match_the_published_schema_and_the_index_is_current]
scope: [docs]
---

# Значение поля с закрытым перечнем входит в перечень

`status` — одно из `active`, `planned`, `superseded`. `governs` — одно из `product`,
`process`. Перечень `scope` README объявляет открытым, и реестр его не сверяет.
