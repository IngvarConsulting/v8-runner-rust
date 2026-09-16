---
id: INV.DOCS.A-SYMBOL-NAMES-EXACTLY-ONE-RECORD
status: active
governs: process
decision: DEC.2026-09-16.A-SYMBOL-NAMES-EXACTLY-ONE-RECORD
check: [tests/arch_registry.rs::a_symbol_names_exactly_one_record, tests/arch_registry.rs::registry_records_match_the_published_schema_and_the_index_is_current]
scope: [docs]
---

# Двух записей с одним символом в реестре нет

Непустой `id` встречается в реестре один раз — в одной записи одного каталога. Вторая
запись с тем же символом отклоняется, каким бы каталогом и видом она ни называлась.
