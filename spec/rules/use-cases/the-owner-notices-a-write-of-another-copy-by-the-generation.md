---
id: INV.USE-CASES.THE-OWNER-NOTICES-A-WRITE-OF-ANOTHER-COPY-BY-THE-GENERATION
check:
  - tests/cli_infobase_owner.rs::the_owner_notices_a_write_of_another_copy_by_the_generation
---

# Владелец замечает запись другой копии по поколению

После записи другой рабочей копии в его базу
(`INV.USE-CASES.A-WRITE-ON-A-BASE-OF-ANOTHER-COPY-RUNS-WITH-A-WARNING`) владелец остаётся
владельцем, а его следующая отправка получает отказ «база ушла вперёд»
(`INV.USE-CASES.A-PUSH-INTO-A-BASE-THAT-MOVED-AHEAD-IS-REFUSED`), а не затирает чужую запись
молча.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).
