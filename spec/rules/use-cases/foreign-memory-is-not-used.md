---
id: INV.USE-CASES.FOREIGN-MEMORY-IS-NOT-USED
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/214
---

# Чужая память не используется

Память под именем базы, записанная для другой базы, объявляется чужой в ответе и не даёт
основания пропустить работу.

Обычный `push` при чужой хеш-памяти отказывает, называя записанную и выбранную привязки
и выходы: полный `pull`, если права база, и `push --full`, если прав каталог; каждый
заменяет память после успеха.

Проверенный срез — отказ при чужой хеш-памяти:
`tests/cli_pull_memory.rs::foreign_memory_is_named_in_the_response_without_dispatching_or_exposing_credentials`
и `src/change_detection/source_sets.rs::base_snapshots_remain_separate_and_reject_a_retargeted_base`.
Привязка остальных видов памяти остаётся в #214.
