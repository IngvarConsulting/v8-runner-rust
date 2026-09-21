---
id: INV.CONFIG.AN-INFOBASE-NAME-IS-A-PLAIN-IDENTIFIER
status: planned
governs: product
decision: DEC.2026-09-21.INFOBASES-ARE-A-NAMED-MAP-WITH-ORIGIN-AS-THE-DEFAULT
check: null
scope: [config]
---

# Имя базы — простой идентификатор

Ключ карты `infobases`, не подходящий под `[A-Za-z0-9][A-Za-z0-9_-]{0,63}`, отвергается
валидацией с указанием ключа; каталог памяти строится из имени без преобразований,
поэтому вывести его за пределы `workPath/infobases/` нельзя.
