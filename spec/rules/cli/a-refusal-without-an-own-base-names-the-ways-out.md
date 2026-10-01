---
id: INV.CLI.A-REFUSAL-WITHOUT-AN-OWN-BASE-NAMES-THE-WAYS-OUT
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/330
---

# Отказ без своей базы называет выходы

Отказ рабочей копии без объявленной базы и отказ на базе другой копии называют выходы: своя
чистая база (`init --infobase`, `infobase create`), копия базы (`infobase create --from`) и
общая база по согласию.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies),
[`cli.html#map`](../../../docs/site/cli.html#map).
