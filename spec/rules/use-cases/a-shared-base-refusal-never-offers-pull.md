---
id: INV.USE-CASES.A-SHARED-BASE-REFUSAL-NEVER-OFFERS-PULL
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/215
---

# Отказ на общей базе выгрузку не предлагает

На общей базе ни отказ первого знакомства, ни отказ «база ушла вперёд» не предлагает `pull`:
выход — `push --force`, и отказ называет остальных владельцев.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies),
[`cli.html#refusals`](../../../docs/site/cli.html#refusals).
