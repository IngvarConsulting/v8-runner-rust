---
id: INV.DOCS.A-CLOSED-FIELD-ADMITS-ONLY-PUBLISHED-VALUES
status: active
governs: process
decision: DEC.2026-09-16.A-CLOSED-FIELD-ADMITS-ONLY-PUBLISHED-VALUES
check: [tests/arch_registry.rs::a_closed_field_admits_only_published_values, tests/arch_registry.rs::registry_records_match_the_published_schema_and_the_index_is_current]
scope: [docs]
---

# Значения `status` и `governs` — из опубликованного перечня

`status` равен `active`, `planned` или `superseded`; `governs` — `product` или
`process`. Значение мимо перечня реестр отклоняет и тогда, когда оно выглядит уместным.
По `status` он ветвит собственные проверки, и незнакомое слово там не нарушает правило,
а отменяет его. Списки `scope` и `consumers` правилом не затронуты.
