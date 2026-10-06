---
id: INV.USE-CASES.A-SHARED-BASE-REFUSAL-OFFERS-PULL-FIRST-AND-NAMES-PUSH
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/215
---

# Отказ на общей базе предлагает выгрузку первой и называет отправку

На общей базе отказ первого знакомства и отказ «база ушла вперёд» называют выходы:
`pull` — безопасный, он стоит следующим шагом, и `push --force`, который называет текст отказа.
Отказ говорит, что базу меняют и другие копии, и называет остальных владельцев.

Решение владельца от 06.10.2026: отказ называет все выходы, которые есть.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies),
[`cli.html#refusals`](../../../docs/site/cli.html#refusals).
