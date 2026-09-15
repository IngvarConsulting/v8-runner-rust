---
id: INV.PLATFORM.A-KEPT-AGENT-YIELDS-THE-INFOBASE
status: planned
governs: product
decision: DEC.2026-09-15.A-KEPT-AGENT-LIVES-WITH-THE-WORKSPACE
check: null
scope: [use-cases, platform]
---

# Живой агент уступает базу другому исполнителю

Перед запуском операции над той же базой другим исполнителем раннер останавливает
своего живого агента и называет остановку в квитанции; чужой агент (`attach`) не
останавливается никогда — вместо этого команда отказывает типизированно.
