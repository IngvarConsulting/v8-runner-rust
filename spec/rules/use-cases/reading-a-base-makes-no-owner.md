---
id: INV.USE-CASES.READING-A-BASE-MAKES-NO-OWNER
check:
  - tests/cli_infobase_owner.rs::a_read_on_a_base_of_another_copy_passes_and_leaves_the_marker
  - tests/cli_infobase_owner.rs::a_read_of_a_base_without_a_marker_makes_no_owner
  - src/use_cases/infobase_owner.rs::only_a_run_of_a_write_on_a_declared_base_records_a_copy
  - tests/cli_status.rs::status_deep_on_a_base_without_a_marker_makes_no_owner
  - tests/cli_infobase_copy.rs::debugging_on_a_copy_of_the_base_leaves_the_neighbour_untouched
---

# Чтение базы владельцем не делает

Команда чтения на базе другой рабочей копии проходит и в метку ничего не пишет. В словаре
сайта это `infobase dump`, `download`, `extensions list`, `diff --against` и `status --deep`,
а кроме того источник `infobase create --from`. Команды записи и чтения определены в
`INV.USE-CASES.A-DEVELOPMENT-BASE-IS-HELD-BY-ONE-WORKING-COPY`.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies).
