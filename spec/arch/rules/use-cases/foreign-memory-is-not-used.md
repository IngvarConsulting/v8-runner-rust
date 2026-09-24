---
id: INV.USE-CASES.FOREIGN-MEMORY-IS-NOT-USED
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/214
---

# Чужая память не используется

Память под именем базы, записанная для другой базы, объявляется чужой в ответе и не даёт
основания пропустить работу.
