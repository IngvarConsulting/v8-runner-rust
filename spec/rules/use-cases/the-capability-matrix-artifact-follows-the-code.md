---
id: INV.USE-CASES.CAPABILITY-MATRIX-ARTIFACT-FOLLOWS-THE-CODE
check: [src/domain/capability.rs::generated_capability_matrix_is_current]
---

# Артефакт матрицы исполнителей равен коду

`docs/schemas/capability-matrix.json` — матрица `src/domain/capability.rs` как данные: для
каждой операции и вида цели исполнители строки в её порядке и признак, входит ли исполнитель
в цепочку умолчаний. Артефакт порождается командой
`UPDATE_CAPABILITY_MATRIX=1 cargo test --bin v8-runner generated_capability_matrix_is_current`
и руками не правится; отставший от кода артефакт валит проверку.

Артефакт нужен потому, что потребителю вне Rust, сверке сайта `scripts/site_matrix.py`,
иначе пришлось бы разбирать исходник: тогда у матрицы появился бы второй владелец, и
перестройка кода без смены её содержания ложно роняла бы сверку.
