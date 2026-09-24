---
id: INV.USE-CASES.OPERATIONS-DECLARE-AN-INTERRUPTION-CLASS
check:
  - src/use_cases/init_project.rs::init_honors_interruption_before_infobase_create_safe_point
  - src/platform/edt.rs::interactive_dsl_preserves_deferred_interruption_for_critical_phase
---

# У операции объявлен класс безопасности прерывания

Каждая операция с внешним процессом называет свой класс прерывания; умолчания «снять немедленно» не существует.
