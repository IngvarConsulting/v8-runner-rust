---
id: INV.PLATFORM.UNREACHABLE-ATTACHED-IS-A-TYPED-REFUSAL
check: [tests/cli_dump_agent.rs::an_unreachable_attached_agent_is_refused_and_no_process_is_launched_instead]
---

# Недоступный чужой процесс даёт отказ, а не подмену

Если объявленная точка входа не отвечает, команда отказывает типизированно и не поднимает собственный процесс рядом.
