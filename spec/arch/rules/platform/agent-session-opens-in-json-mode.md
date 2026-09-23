---
id: INV.PLATFORM.AGENT-SESSION-OPENS-IN-JSON-MODE
check: [tests/cli_dump_agent.rs::managed_agent_dumps_through_the_built_in_ssh_client_and_reads_the_result_from_disk]
---

# Первая команда сессии переводит её в машинный режим

Сессия агента начинается с установки формата ответа в JSON и отключения приглашения; до этого команды не отправляются.
