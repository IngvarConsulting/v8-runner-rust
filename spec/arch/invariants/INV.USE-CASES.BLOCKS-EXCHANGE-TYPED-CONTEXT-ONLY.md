---
id: INV.USE-CASES.BLOCKS-EXCHANGE-TYPED-CONTEXT-ONLY
status: active
governs: process
decision: DEC.2026-04-21.A-COMMAND-IS-A-PIPELINE-OF-TYPED-BLOCKS
check: tests/use_case_boundaries.rs::use_cases_keep_no_hidden_shared_state
scope: [use-cases]
---

# Блоки обмениваются только явным типизированным контекстом

Промежуточные пути, артефакты, выбранный набор и разобранный вывод идут через объявленный контекст, а не через скрытое общее состояние.
