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

Потребитель вне Rust читает матрицу только из артефакта, а не из исходника: так сверка сайта
`scripts/site_matrix.py` не становится вторым владельцем матрицы, и перестройка кода матрицы
без смены её содержания сверку не задевает.
