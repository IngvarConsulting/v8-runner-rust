---
id: INV.PLATFORM.UNREACHABLE-ATTACHED-IS-A-TYPED-REFUSAL
status: active
governs: product
decision: DEC.2026-09-14.AGENT-ENDPOINT-IS-MANAGED-OR-ATTACHED
check: tests/cli_dump_agent.rs::an_unreachable_attached_agent_is_refused_and_no_process_is_launched_instead
scope: [platform]
---

# Недоступный чужой процесс даёт отказ, а не подмену

Если объявленная точка входа не отвечает, команда отказывает типизированно и не поднимает собственный процесс рядом.
