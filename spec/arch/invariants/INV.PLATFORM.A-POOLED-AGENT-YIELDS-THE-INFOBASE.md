---
id: INV.PLATFORM.A-POOLED-AGENT-YIELDS-THE-INFOBASE
status: planned
governs: product
decision: DEC.2026-09-15.AGENTS-LIVE-IN-ONE-DAEMONS-POOL-KEYED-BY-THE-INFOBASE
check: null
scope: [use-cases, platform]
---

# Агент из пула уступает базу другому исполнителю

Перед операцией над той же базой другим исполнителем (`ibcmd`, пакетный Конфигуратор) команда просит демон погасить запись этой базы и называет остановку в квитанции; чужой агент (`attach`) не гасится никогда — команда отказывает типизированно.
