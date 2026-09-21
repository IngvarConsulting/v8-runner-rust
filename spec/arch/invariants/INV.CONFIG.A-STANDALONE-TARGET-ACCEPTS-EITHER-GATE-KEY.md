---
id: INV.CONFIG.A-STANDALONE-TARGET-ACCEPTS-EITHER-GATE-KEY
status: planned
governs: product
decision: DEC.2026-09-21.A-STANDALONE-TARGET-HAS-TWO-GATES
check: null
scope: [config]
---

# Автономной цели достаточно любого из двух ключей

Секция базы с объявленной секцией `standalone` проходит валидацию и с одной строкой
подключения прямого шлюза в `connection`, и с одним `standalone.gate`; оба ключа
расширяют набор операций, и ни один не обязателен при наличии другого.
