---
id: INV.DOCS.A-FIELD-HAS-THE-SHAPE-THE-README-PUBLISHES
status: active
governs: process
decision: DEC.2026-09-16.A-FIELD-HAS-THE-SHAPE-THE-README-PUBLISHES
check: [tests/arch_registry.rs::a_field_has_the_shape_the_readme_publishes, tests/arch_registry.rs::registry_records_match_the_published_schema_and_the_index_is_current]
scope: [docs]
---

# Перечень записан перечнем, символ — символом

`supersedes`, `establishes`, `changes`, `scope` и `consumers` несут перечень. `id`,
`status`, `governs`, `version`, `artifact`, `producer`, `decision` и `superseded-by`
несут одно значение. `check` и `realized` принимают и адрес, и перечень адресов.
