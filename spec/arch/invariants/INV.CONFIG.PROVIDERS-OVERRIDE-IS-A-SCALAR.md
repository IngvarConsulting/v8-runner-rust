---
id: INV.CONFIG.PROVIDERS-OVERRIDE-IS-A-SCALAR
status: active
governs: product
decision: DEC.2026-09-14.PROVIDER-OVERRIDE-IS-STRICT
check: [tests/provider_matrix.rs::a_provider_override_is_a_scalar_for_an_operation_with_a_choice, src/config/validate.rs::provider_overrides_are_checked_against_the_matrix]
scope: [config]
---

# Переопределение провайдера — скаляр и только для операции с развилкой

Список значений, операция без развилки и провайдер вне матрицы отклоняются на границе валидации.
