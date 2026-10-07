---
id: INV.USE-CASES.A-COPIED-BASE-STARTS-WITH-A-FULL-PUSH
check:
  - tests/cli_infobase_copy.rs::a_copied_base_starts_with_a_full_push
  - tests/cli_infobase_copy.rs::debugging_on_a_copy_of_the_base_leaves_the_neighbour_untouched
  - src/use_cases/exchange_guard.rs::a_copy_mark_is_memory_and_offers_no_pull
---

# После копии базы первая отправка полная

После `infobase create --from` память рабочей копии знает только поколение новой базы и то,
что содержимое пришло из другой базы: признак копии под памятью базы, а прежние хеш-память,
копии файла версий и журнал поколений под её именем стёрты. Признак копии — память каждого
набора (`INV.USE-CASES.WHAT-COUNTS-AS-MEMORY-OF-THE-BASE`), поэтому отказ первого знакомства
первую отправку не останавливает, а сама она идёт полной. Первая удачная отправка признак
снимает; база, собранная `infobase create` из исходников, — тоже.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies),
[`cli.html#refusals`](../../../docs/site/cli.html#refusals).
