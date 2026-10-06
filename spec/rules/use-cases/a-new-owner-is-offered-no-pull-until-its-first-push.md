---
id: INV.USE-CASES.A-NEW-OWNER-IS-OFFERED-NO-PULL-UNTIL-ITS-FIRST-PUSH
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/215
---

# Новому владельцу выгрузку не предлагают до первой отправки

Если рабочая копия взяла базу без метки или сменила ушедшего владельца, а база ушла вперёд её
памяти, до первой удачной отправки ни один ответ не предлагает `pull`. Выход — `push --force`,
и отказ говорит, что базу могли менять другие копии.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).
