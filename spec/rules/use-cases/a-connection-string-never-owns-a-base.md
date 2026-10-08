---
id: INV.USE-CASES.A-CONNECTION-STRING-NEVER-OWNS-A-BASE
check:
  - tests/cli_infobase_owner.rs::a_connection_string_never_owns_and_warns_on_a_base_of_another_copy
  - src/use_cases/infobase_owner.rs::only_a_run_of_a_write_on_a_declared_base_records_a_copy
---

# Строка соединения владельцем не становится

Команда записи со строкой соединения в `--infobase` свою копию в метку не записывает ни на
какой базе, в том числе без метки или с ушедшим владельцем. На файловой базе другой рабочей
копии она идёт с тем же предупреждением, что и с именем базы
(`INV.USE-CASES.A-WRITE-ON-A-BASE-OF-ANOTHER-COPY-RUNS-WITH-A-WARNING`). Команды записи и
чтения определены в `INV.USE-CASES.A-DEVELOPMENT-BASE-IS-HELD-BY-ONE-WORKING-COPY`.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).
