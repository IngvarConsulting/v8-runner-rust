---
id: INV.PLATFORM.A-MANAGED-AGENT-IS-PINNED-TO-A-ONE-TIME-KEY
check:
  - tests/cli_dump_agent.rs::managed_agent_dumps_through_the_built_in_ssh_client_and_reads_the_result_from_disk
  - tests/cli_dump_agent.rs::a_managed_agent_without_a_declared_key_refuses_a_foreign_key_on_its_port
  - tests/cli_dump_agent.rs::a_managed_agent_is_pinned_by_the_host_key_file_it_was_given
---

# Управляемый агент закреплён на ключе, который раннер ему отдал

Агент, которого раннер поднимает сам, всегда получает ключ хоста ключом запуска
`/AgentSSHHostKey`, и сессия открывается только с этим ключом. Объявлен
`tools.designer_agent.host-key` — это он. Не объявлен — раннер создаёт одноразовый
ED25519-ключ на этот запуск под `workPath/agent/`, отдаёт агенту его файл и удаляет файл
вместе с агентом; ключа платформы (`/AgentSSHHostKeyAuto`) раннер не берёт.

SSH-сервер, который ответил на порту агента другим ключом, — типизированный отказ
`environment_unavailable`, называющий оба отпечатка; ни одной команды такому серверу раннер
не отдаёт.
