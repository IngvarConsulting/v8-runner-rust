---
id: INV.USE-CASES.FOREIGN-MEMORY-IS-NOT-USED
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/214
---

# Чужая память не используется

Память под именем базы, записанная для другой базы, объявляется чужой в ответе и не даёт
основания пропустить работу.

Для хешей наборов исходников это закреплено правилом
`INV.USE-CASES.HASH-MEMORY-IS-SCOPED-TO-ITS-BASE-AND-SOURCE`. Привязка остальных видов
памяти остаётся в #214.
