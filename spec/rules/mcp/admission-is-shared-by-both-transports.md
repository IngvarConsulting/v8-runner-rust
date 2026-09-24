---
id: INV.MCP.ADMISSION-IS-SHARED-BY-BOTH-TRANSPORTS
check: [tests/architecture_guardrails.rs::mcp_admission_is_built_once_and_shared_by_both_transports]
---

# Лимит одновременных вызовов общий для обоих транспортов

Ограничение допуска применяется одинаково к обоим транспортам и различает ожидание слота и исполнение.
