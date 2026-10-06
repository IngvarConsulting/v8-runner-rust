---
id: INV.USE-CASES.AN-UNREADABLE-MARKER-STOPS-A-WRITE
check:
  - tests/cli_infobase_owner.rs::an_unreadable_marker_stops_a_write_and_a_read_goes_on
  - src/use_cases/infobase_owner.rs::a_marker_that_cannot_be_written_stops_a_write
---

# Нечитаемая метка останавливает команду записи

Если метку нельзя прочитать или записать, команда записи отказывает и называет каталог и
причину, а команда чтения идёт дальше и говорит об этом. Метки ещё нет — это не отказ: её
заводит первая копия. Команды записи и чтения определены в
`INV.USE-CASES.A-DEVELOPMENT-BASE-IS-HELD-BY-ONE-WORKING-COPY`.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).
