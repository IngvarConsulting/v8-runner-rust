---
id: INV.CONFIG.A-STANDALONE-TARGET-ACCEPTS-EITHER-GATE-KEY
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/205
---

# Автономной цели достаточно любого из двух ключей

Секция базы с объявленной секцией `standalone` проходит валидацию и с одной строкой
подключения прямого шлюза в `connection`, и с одним `standalone.gate`; оба ключа
расширяют набор операций, и ни один не обязателен при наличии другого.
