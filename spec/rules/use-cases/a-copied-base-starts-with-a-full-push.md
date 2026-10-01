---
id: INV.USE-CASES.A-COPIED-BASE-STARTS-WITH-A-FULL-PUSH
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/330
---

# После копии базы первая отправка полная

После `infobase create --from` память рабочей копии знает только поколение новой базы и то,
что содержимое пришло из другой базы, поэтому первая отправка идёт полной, и отказ первого
знакомства её не останавливает.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies),
[`cli.html#refusals`](../../../docs/site/cli.html#refusals).
