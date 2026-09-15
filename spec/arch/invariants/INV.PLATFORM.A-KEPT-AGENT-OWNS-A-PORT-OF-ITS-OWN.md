---
id: INV.PLATFORM.A-KEPT-AGENT-OWNS-A-PORT-OF-ITS-OWN
status: planned
governs: product
decision: DEC.2026-09-15.A-KEPT-AGENT-LIVES-WITH-THE-WORKSPACE
check: null
scope: [platform, config]
---

# У живого агента свой порт, выбранный при старте

Агент с временем жизни `workspace` не берёт умолчание `1543`: он поднимается на
свободном порту `127.0.0.1`, записывает его в удостоверение, и два рабочих пространства
на одной машине никогда не делят порт. Явный `port` в конфиге остаётся законным и
объявляет намерение владельца.
