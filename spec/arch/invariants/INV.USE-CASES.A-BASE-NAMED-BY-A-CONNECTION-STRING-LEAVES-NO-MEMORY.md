---
id: INV.USE-CASES.A-BASE-NAMED-BY-A-CONNECTION-STRING-LEAVES-NO-MEMORY
status: planned
governs: product
decision: DEC.2026-09-21.MEMORY-IS-KEPT-PER-INFOBASE
check: null
scope: [use-cases, cli]
---

# База, названная строкой соединения, памяти не оставляет

После команды с `--infobase <строка соединения>` под `workPath/infobases/` не появляется
ни каталога, ни записи, и следующая команда с той же строкой идёт как первая: `pull`
полный, `push` в непустую базу — отказ с выходами `pull` и `push --force`.
