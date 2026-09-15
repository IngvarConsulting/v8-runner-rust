---
id: INV.PLATFORM.A-POOL-ENTRY-DIES-BY-ITS-OWN-IDLE-TIMEOUT
status: planned
governs: product
decision: DEC.2026-09-15.AGENTS-LIVE-IN-ONE-DAEMONS-POOL-KEYED-BY-THE-INFOBASE
check: null
scope: [platform]
---

# Каждая запись пула стареет отдельно

Простой записи считается от последней команды, прошедшей через неё; по истечении `tools.designer_agent.idle-timeout` (умолчание 30 минут) демон гасит только этого агента, не трогая остальных; у записи чужого агента гасится лишь сессия. Запись гасится и раньше — по `agent stop <база>` и когда файл базы или её платформа исчезли.
