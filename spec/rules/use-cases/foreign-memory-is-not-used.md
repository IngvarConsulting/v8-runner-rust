---
id: INV.USE-CASES.FOREIGN-MEMORY-IS-NOT-USED
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/214
---

# Чужая память не используется

Память под именем базы, записанная для другой базы, объявляется чужой в ответе и не даёт
основания пропустить работу.

Для хешов привязка и содержимое снимка записываются атомарно. Обычный `push` при чужой
хеш-памяти отказывает с прежней привязкой и предлагает полный `pull`; явная полная
загрузка заменяет её после успеха.

Проверенный срез — отказ при чужой хеш-памяти:
`tests/cli_pull_memory.rs::foreign_memory_is_named_in_the_response_without_dispatching_or_exposing_credentials`
и `src/change_detection/source_sets.rs::base_snapshots_remain_separate_and_reject_a_retargeted_base`.
Привязка остальных видов памяти остаётся в #214.
