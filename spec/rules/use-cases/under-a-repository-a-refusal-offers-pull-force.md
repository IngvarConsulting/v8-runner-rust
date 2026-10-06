---
id: INV.USE-CASES.UNDER-A-REPOSITORY-A-REFUSAL-OFFERS-PULL-FORCE
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/220
---

# Под хранилищем отказ предлагает `pull --force`

В базе под хранилищем конфигурации полная загрузка невозможна, поэтому отказы первого
знакомства и «база ушла вперёд» называют вместо `push --force` выход `pull --force` — и на
общей базе тоже. Что база под хранилищем, раннер до загрузки пока не знает.

Источник: [`cli.html#refusals`](../../../docs/site/cli.html#refusals).
