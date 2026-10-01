---
id: INV.CLI.A-BASE-LOCK-LIVES-WITH-ITS-COMMAND
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/326
---

# Замок базы живёт, пока жива команда

Замок файловой базы, оставленный убитой командой, следующую команду не останавливает.

Источник: [`architecture.html#life`](../../../docs/site/architecture.html#life).
