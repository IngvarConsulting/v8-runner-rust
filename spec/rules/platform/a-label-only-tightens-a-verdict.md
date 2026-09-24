---
id: INV.PLATFORM.A-LABEL-ONLY-TIGHTENS-A-VERDICT
check:
  - src/use_cases/check_syntax.rs::labels_can_only_make_a_verdict_stricter
  - src/use_cases/check_syntax.rs::an_unrecognised_severity_is_an_error_not_a_warning
---

# Помета делает вердикт строже и никогда мягче

Помета, снятая с прозы находки, не способна смягчить вердикт, а неопознанная степень важности считается ошибкой, а не предупреждением.
