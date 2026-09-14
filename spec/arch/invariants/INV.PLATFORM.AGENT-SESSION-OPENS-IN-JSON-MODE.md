---
id: INV.PLATFORM.AGENT-SESSION-OPENS-IN-JSON-MODE
status: planned
governs: product
decision: DEC.2026-09-14.AGENT-SPEAKS-JSON-WITHOUT-A-PTY
check: null
scope: [platform]
---

# Первая команда сессии переводит её в машинный режим

Сессия агента начинается с установки формата ответа в JSON и отключения приглашения; до этого команды не отправляются.
