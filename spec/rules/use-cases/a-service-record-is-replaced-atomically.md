---
id: INV.USE-CASES.A-SERVICE-RECORD-IS-REPLACED-ATOMICALLY
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/536
---

# Запись о службе заменяется атомарно

Читатель видит старую или новую запись целиком. Пока служба запускается, запись несёт состояние `starting`.
