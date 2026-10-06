---
id: INV.CONFIG.PARTIAL-LOAD-THRESHOLD-KEY-IS-REJECTED
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/379
---

# Ключ порога частичной загрузки отклоняется по имени

Ключ `push.partialLoadThreshold` — и в прежней секции `build` — не принимают ни проектный
файл, ни местный слой. Отказ называет ключ, говорит, что порога больше нет и строку нужно
удалить, а полную загрузку по желанию даёт `push --full`. `init` и `clone` этот ключ не пишут.
