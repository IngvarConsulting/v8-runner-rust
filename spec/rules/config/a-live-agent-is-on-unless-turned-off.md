---
id: INV.CONFIG.A-LIVE-AGENT-IS-ON-UNLESS-TURNED-OFF
check:
  - tests/cli_live_agent.rs::live_off_removes_the_live_agent_and_works_one_shot
  - tests/cli_live_agent_portable.rs::live_off_removes_the_live_agent_on_any_os
  - tests/cli_live_agent_portable.rs::agent_hold_refuses_with_live_off_and_over_the_limit
  - src/config/loader.rs::live_max_zero_is_refused_and_names_live_false
  - src/config/loader.rs::live_keys_next_to_attach_are_refused
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
