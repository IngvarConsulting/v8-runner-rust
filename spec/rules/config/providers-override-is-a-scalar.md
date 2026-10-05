---
id: INV.CONFIG.PROVIDERS-OVERRIDE-IS-A-SCALAR
check:
  - tests/provider_matrix.rs::a_provider_override_is_a_scalar_for_an_operation_with_a_choice
  - src/config/validate.rs::provider_overrides_are_checked_against_the_matrix
  - tests/contract_receipt.rs::an_override_names_its_file_and_never_falls_back
  - tests/cli_infobase.rs::an_override_does_not_fall_back_when_its_provider_is_missing
  - tests/provider_matrix.rs::a_foreign_download_provider_is_refused_like_push_before_the_platform_starts
  - tests/provider_matrix.rs::a_foreign_operation_key_does_not_block_the_transfer_family
  - tests/provider_matrix.rs::a_transfer_command_checks_only_the_key_of_its_own_operation
---

# Переопределение провайдера — скаляр и только для операции с развилкой

Список значений, операция без развилки и провайдер вне матрицы отклоняются на границе
валидации. Названный ключом провайдер умолчанием не подменяется: не готов он — команда
отказывает, а квитанция называет файл, откуда пришёл ключ.
Команда семейства переноса судит только ключ своей операции: ключ любой другой операции её не
останавливает.
