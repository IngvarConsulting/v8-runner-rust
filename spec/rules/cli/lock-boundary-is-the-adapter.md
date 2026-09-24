---
id: INV.CLI.LOCK-BOUNDARY-IS-THE-ADAPTER
check: [tests/architecture_guardrails.rs::public_command_adapters_keep_workspace_lock_boundary]
---

# Блокировку берёт адаптер команды, а не сценарий

Публичная команда, работающая с состоянием под `workPath`, захватывает блокировку на границе адаптера; вложенные шаги идут под ней.
