---
id: INV.DOCS.BOTH-GATES-READ-ONE-FRONT-MATTER-FORM
status: active
governs: process
decision: DEC.2026-09-16.FRONT-MATTER-HAS-ONE-READER
check:
  - tests/arch_registry.rs::both_gates_read_one_front_matter_form
  - tests/arch_registry.rs::a_list_reads_the_same_in_both_forms
scope: [docs, ci]
---

# Оба гейта читают один вид полей

`scripts/arch/registry.py` и `tests/arch_registry.rs` разбирают блок полей одинаково:
на одном и том же тексте они дают одни и те же поля либо оба отказывают. Плоский
список пишется потоковым видом `[a, b]` и блочным, и значение от вида не зависит.
