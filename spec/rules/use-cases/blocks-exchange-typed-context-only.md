---
id: INV.USE-CASES.BLOCKS-EXCHANGE-TYPED-CONTEXT-ONLY
check:
  - tests/use_case_boundaries.rs::use_cases_keep_no_hidden_shared_state
  - tests/use_case_boundaries.rs::the_shared_state_guard_allows_only_declared_context_storage
---

# Блоки обмениваются только явным типизированным контекстом

Промежуточные пути, артефакты, выбранный набор и разобранный вывод идут через объявленный контекст, а не через скрытое общее состояние.
