---
id: INV.USE-CASES.A-TEST-WITHOUT-MEMORY-IS-REFUSED-LIKE-A-PUSH
check:
  - tests/cli_test.rs::a_test_on_a_base_without_memory_is_refused_like_a_push
---

# `test` без памяти о базе отказывает, как `push`

`test`, который перед прогоном грузит исходники в базу, отказывает на базе без памяти тем же
отказом `no_memory` с теми же выходами, что `push`, и платформу не запускает. Его шаг сборки
называет этот отказ.

Решение владельца от 06.10.2026.
