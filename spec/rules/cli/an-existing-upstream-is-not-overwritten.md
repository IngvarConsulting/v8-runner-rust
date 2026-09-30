---
id: INV.CLI.AN-EXISTING-UPSTREAM-IS-NOT-OVERWRITTEN
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/329
---

# Существующий `upstream` не перезаписывается

Если в местном слое уже есть `upstream`, `init --infobase` с новым адресом отказывает и
называет `origin` и `upstream`.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies),
[`cli.html#map`](../../../docs/site/cli.html#map).
