---
id: INV.PLATFORM.LOCAL-AGENT-READS-RESULTS-FROM-DISK
status: active
governs: product
decision: DEC.2026-09-14.AGENT-ENDPOINT-IS-MANAGED-OR-ATTACHED
check: tests/cli_dump_agent.rs::managed_agent_dumps_through_the_built_in_ssh_client_and_reads_the_result_from_disk
scope: [platform]
---

# Локальный агент отдаёт файлы через диск, а не по SFTP

У поднятого раннером агента базовый каталог лежит в `workPath`
(`<workPath>/agent/base`), и результат читается с диска по раскладке платформы:
`agentbasedir.json` называет каталог пользователя, команды пишут относительно него. Правило держится ровно потому, что такой агент по
`INV.PLATFORM.MANAGED-MODE-REQUIRES-A-LOCAL-ENDPOINT` всегда локален; к агенту,
поднятому на другой машине, оно не относится.
