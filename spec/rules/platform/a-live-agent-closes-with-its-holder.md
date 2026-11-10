---
id: INV.PLATFORM.A-LIVE-AGENT-CLOSES-WITH-ITS-HOLDER
check:
  - tests/agent_keeper.rs::the_agent_closes_when_its_holder_process_exits
  - tests/agent_keeper.rs::a_ttl_holder_expires_and_takes_the_agent_with_it
  - tests/cli_live_agent_portable.rs::a_receipt_says_removed_when_the_last_holder_left_during_the_command
---

# Живой агент уходит вместе с последним держателем

Держатель с процессом, например демон Unica, держит агента, пока его процесс жив. Агент
отпускают и снимают сразу, без ожидания простоя, если пропал последний держатель:

- процесс держателя завершился;
- держатель со сроком не продлил удержание вовремя (`INV.PLATFORM.AN-AGENT-HOLDER-IS-A-LIVE-PROCESS`).
