---
id: INV.CONFIG.PROVIDERS-OVERRIDE-IS-A-SCALAR
check:
  - tests/provider_matrix.rs::a_provider_override_is_a_scalar_for_an_operation_with_a_choice
  - src/config/validate.rs::provider_overrides_are_checked_against_the_matrix
---

# Переопределение провайдера — скаляр и только для операции с развилкой

Список значений, операция без развилки и провайдер вне матрицы отклоняются на границе валидации.
