---
id: INV.PLATFORM.THE-DAEMON-EXITS-WITH-AN-EMPTY-POOL
status: planned
governs: product
decision: DEC.2026-09-15.AGENTS-LIVE-IN-ONE-DAEMONS-POOL-KEYED-BY-THE-INFOBASE
check: null
scope: [platform]
---

# Демон завершается, когда пул опустел

Демон живёт ровно столько, сколько в пуле есть хотя бы одна запись; погасив последнюю, он завершается сам и убирает свой сокет. Автозапуска при входе нет: первая команда, которой нужен агент, поднимает демон заново.
