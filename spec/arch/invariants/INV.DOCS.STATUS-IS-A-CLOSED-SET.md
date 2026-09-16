---
id: INV.DOCS.STATUS-IS-A-CLOSED-SET
status: active
governs: process
decision: DEC.2026-09-16.STATUS-IS-A-CLOSED-SET
check: [tests/arch_registry.rs::status_reads_active_planned_or_superseded, tests/arch_registry.rs::registry_records_match_the_published_schema_and_the_index_is_current]
scope: [docs]
---

# Значение `status` входит в опубликованный перечень

`status` любой записи — `active`, `planned` или `superseded`. Слово вне перечня
отклоняется; пустое значение называет проверка обязательных полей.
