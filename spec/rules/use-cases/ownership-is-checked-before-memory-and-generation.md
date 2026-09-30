---
id: INV.USE-CASES.OWNERSHIP-IS-CHECKED-BEFORE-MEMORY-AND-GENERATION
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/327
---

# Сначала владелец, затем память, затем поколение

Проверки перед обменом с базой идут по порядку: чья база, есть ли о ней память, не ушла ли
она вперёд. Отказ называет первую непройденную.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).
