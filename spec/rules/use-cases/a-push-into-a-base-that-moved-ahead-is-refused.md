---
id: INV.USE-CASES.A-PUSH-INTO-A-BASE-THAT-MOVED-AHEAD-IS-REFUSED
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/215
---

# Отправка в ушедшую вперёд базу отклоняется

`push` без `--force`, увидев поколение базы, отличное от записанного в памяти,
отказывает родом `non_fast_forward` до загрузки и называет оба поколения и следующий
шаг.
