---
id: INV.PLATFORM.AGENT-SESSION-OPENS-IN-JSON-MODE
status: active
governs: product
decision: DEC.2026-09-15.AGENT-IS-DRIVEN-BY-THE-SYSTEM-SSH-CLIENT
check: tests/cli_dump_agent.rs::managed_agent_dumps_through_the_system_ssh_client_and_reads_the_result_from_disk
scope: [platform]
---

# Первая команда сессии переводит её в машинный режим

Сессия агента начинается с установки формата ответа в JSON и отключения приглашения; до этого команды не отправляются.
