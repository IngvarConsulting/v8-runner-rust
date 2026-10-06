---
id: INV.USE-CASES.A-NEW-OWNER-IS-OFFERED-NO-PULL-UNTIL-ITS-FIRST-PUSH
check:
  - tests/cli_push_generation.rs::a_new_owner_is_offered_no_pull_until_its_first_push
---

# Новому владельцу выгрузку не предлагают до первой отправки

Если рабочая копия взяла базу без метки или сменила ушедшего владельца, а база ушла вперёд её
памяти, до первой удачной отправки ни один ответ не предлагает `pull`. Выход — `push --force`,
и отказ говорит, что базу могли менять другие копии.

Признак нового владельца лежит в памяти копии о базе под `workPath`, а не в метке: его пишет
граница команды вместе со взятием базы, снимают первая удачная загрузка набора и создание
базы раннером. Общей базе выгрузку предлагают всегда
(`INV.USE-CASES.A-SHARED-BASE-REFUSAL-OFFERS-PULL-FIRST-AND-NAMES-PUSH`).

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).
