---
id: INV.CONFIG.A-LIVE-AGENT-IS-ON-UNLESS-TURNED-OFF
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/577
---

# Живой агент включён, пока его не выключили

Пока в конфигурации нет ключа `tools.designer_agent.live`, процесс агента переживает
команду (`INV.PLATFORM.A-MANAGED-AGENT-OUTLIVES-THE-COMMAND`). С `live: false` раннер
поступает иначе:

- команда, как раньше, снимает своего агента после работы;
- живого агента своей рабочей копии, если такой остался, она тоже снимает;
- `agent hold` отказывает с кодом `live_agent_off` и платформу не запускает.

Предел числа живых агентов задаёт отдельный ключ `tools.designer_agent.live-max`
(`INV.CLI.AN-AGENT-HOLD-OVER-THE-LIMIT-IS-REFUSED`).
