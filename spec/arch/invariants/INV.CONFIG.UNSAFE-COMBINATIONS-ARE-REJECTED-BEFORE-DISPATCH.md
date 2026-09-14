---
id: INV.CONFIG.UNSAFE-COMBINATIONS-ARE-REJECTED-BEFORE-DISPATCH
status: planned
governs: product
decision: DEC.2026-04-20.V8PROJECT-YAML-IS-THE-PROJECT-CONTRACT
check: null
scope: [config]
---

# Неподдержанное сочетание отклоняется до вызова платформы

Валидация конфига заканчивается отказом раньше, чем запускается любая утилита: цена ошибки не должна включать частично сделанную работу.
