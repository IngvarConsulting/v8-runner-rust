---
id: INV.PLATFORM.AN-AGENT-IGNORING-SHUTDOWN-IS-NOT-AWAITED
check:
  - src/platform/agent.rs::only_a_platform_known_to_ignore_shutdown_is_not_awaited
  - tests/cli_agent_scenarios.rs::an_agent_of_a_platform_ignoring_shutdown_is_not_awaited
---

# Агента, который не выполняет завершение, не ждут

Управляемый агент версий 8.3.24–8.3.26 получает `common shutdown`, но раннер ждёт ответа и выхода
процесса только коротким сроком и снимает процесс: на этих версиях команда агента не завершает.
Версию раннер берёт из пути установки. Прочие версии и версия, которую путь не называет, получают
полные сроки закрытия.
