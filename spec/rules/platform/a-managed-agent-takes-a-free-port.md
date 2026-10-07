---
id: INV.PLATFORM.A-MANAGED-AGENT-TAKES-A-FREE-PORT
check:
  - tests/cli_bootstrap.rs::bootstrap_empty_dir_creates_config_and_dumps_main_configuration
---

# Порт управляемого агента по умолчанию — свободный на этот запуск

Без `tools.designer_agent.port` раннер берёт у системы свободный порт на `127.0.0.1` и
передаёт его агенту в `/AgentPort`; фиксированного порта по умолчанию нет. Объявленный порт
передаётся как есть.
