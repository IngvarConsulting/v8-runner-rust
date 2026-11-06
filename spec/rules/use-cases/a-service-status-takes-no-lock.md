---
id: INV.USE-CASES.A-SERVICE-STATUS-TAKES-NO-LOCK
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/536
---

# Состояние службы читается без замка

`status` читает запись без замка `workPath`, чтобы отвечать и пока идёт другая команда.
