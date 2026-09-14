---
id: INV.PLATFORM.LOCAL-AGENT-READS-RESULTS-FROM-DISK
status: planned
governs: product
decision: DEC.2026-09-14.AGENT-ENDPOINT-IS-MANAGED-OR-ATTACHED
check: null
scope: [platform]
---

# Локальный агент отдаёт файлы через диск, а не по SFTP

У поднятого раннером агента базовый каталог равен `workPath`, и результат читается с
диска по известной раскладке. Правило держится ровно потому, что такой агент по
`INV.PLATFORM.MANAGED-MODE-REQUIRES-A-LOCAL-ENDPOINT` всегда локален; к агенту,
поднятому на другой машине, оно не относится.
