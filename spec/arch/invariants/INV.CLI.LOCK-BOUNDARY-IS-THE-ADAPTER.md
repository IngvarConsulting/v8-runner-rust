---
id: INV.CLI.LOCK-BOUNDARY-IS-THE-ADAPTER
status: active
governs: product
decision: DEC.2026-04-20.A-COMMAND-OWNS-THE-WORKPATH-EXCLUSIVELY
check: tests/architecture_guardrails.rs::public_command_adapters_keep_workspace_lock_boundary
scope: [cli, mcp]
---

# Блокировку берёт адаптер команды, а не сценарий

Публичная команда, работающая с состоянием под `workPath`, захватывает блокировку на границе адаптера; вложенные шаги идут под ней.
