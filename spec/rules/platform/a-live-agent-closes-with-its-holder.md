---
id: INV.PLATFORM.A-LIVE-AGENT-CLOSES-WITH-ITS-HOLDER
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/577
---

# Живой агент уходит вместе с последним держателем

Держатель с процессом, например демон Unica, держит агента, пока его процесс жив. Агент
отпускают и снимают сразу, без ожидания простоя, если пропал последний держатель:

- процесс держателя завершился;
- держатель со сроком не продлил удержание вовремя (`INV.PLATFORM.AN-AGENT-HOLDER-IS-A-LIVE-PROCESS`).
