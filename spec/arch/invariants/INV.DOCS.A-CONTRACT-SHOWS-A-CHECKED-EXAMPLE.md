---
id: INV.DOCS.A-CONTRACT-SHOWS-A-CHECKED-EXAMPLE
status: active
governs: process
decision: DEC.2026-09-14.A-CONTRACT-SHOWS-A-CHECKED-EXAMPLE
check: [tests/arch_registry.rs::every_contract_shows_an_example_checked_against_its_form, tests/arch_registry.rs::registry_records_match_the_published_schema_and_the_index_is_current]
scope: [docs]
---

# Пример контракта проходит его же форму

У каждого контракта есть раздел «Пример» с блоком кода. Пример из него либо проходит
схему, названную пропом `artifact`, либо является фрагментом закреплённого документа,
если артефакт схемой не является.
