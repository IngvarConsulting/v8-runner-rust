---
id: INV.CLI.WITHOUT-ITS-LOCK-A-WRITE-IS-REFUSED
check:
  - tests/cli_infobase_lock.rs::a_write_without_the_base_lock_is_refused_and_a_read_goes_on_with_a_warning
  - src/use_cases/infobase_lock.rs::a_base_lock_that_cannot_be_taken_refuses_a_write_and_warns_a_read
---

# Без замка базы команда записи отказывает

Если замок файловой базы нельзя взять не потому, что его держит другая команда, команда
записи отказывает и называет каталог и причину, а команда чтения идёт дальше и говорит об
этом. Команды записи и чтения определены в
`INV.USE-CASES.A-DEVELOPMENT-BASE-IS-HELD-BY-ONE-WORKING-COPY`.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).
