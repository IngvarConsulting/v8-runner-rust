---
id: INV.USE-CASES.AN-UNREADABLE-MARKER-STOPS-A-WRITE
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/327
---

# Нечитаемая метка останавливает команду записи

Если метку нельзя прочитать или записать, команда записи отказывает и называет каталог и
причину, а команда чтения идёт дальше и говорит об этом. Метки ещё нет — это не отказ: её
заводит первая копия. Команды записи и чтения определены в
`INV.USE-CASES.A-DEVELOPMENT-BASE-IS-HELD-BY-ONE-WORKING-COPY`.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).
