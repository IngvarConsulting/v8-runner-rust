---
id: INV.PLATFORM.A-POOLED-AGENT-OWNS-A-PORT-OF-ITS-OWN
status: planned
governs: product
decision: DEC.2026-09-15.AGENTS-LIVE-IN-ONE-DAEMONS-POOL-KEYED-BY-THE-INFOBASE
check: null
scope: [platform]
---

# У каждого агента пула свой порт

Демон поднимает каждого агента на свободном порту `127.0.0.1` и хранит порт в записи; умолчание `1543` в пуле не используется, и два агента никогда не делят порт.
