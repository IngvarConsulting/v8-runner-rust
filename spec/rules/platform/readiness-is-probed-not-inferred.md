---
id: INV.PLATFORM.READINESS-IS-PROBED-NOT-INFERRED
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/233
---

# Готовность проверяется перед использованием, а не выводится

Каждый путь проверяется своей пробой непосредственно перед работой; из флагов запуска чужого процесса готовность не выводится.
