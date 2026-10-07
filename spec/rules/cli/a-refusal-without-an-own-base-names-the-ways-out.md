---
id: INV.CLI.A-REFUSAL-WITHOUT-AN-OWN-BASE-NAMES-THE-WAYS-OUT
check:
  - tests/cli_infobases.rs::a_refusal_without_origin_names_the_ways_out
  - tests/cli_infobase_owner.rs::a_write_on_a_base_of_another_copy_is_refused_and_names_the_owner
---

# Отказ без своей базы называет выходы

Отказ рабочей копии без объявленной базы и отказ на базе другой копии называют выходы: своя
чистая база (`init --infobase`, `infobase create`), копия базы (`infobase create --from`) и
база, развёрнутая из эталонного образа (`infobase restore --create`). Общую базу выходом они
не называют.

Решение владельца от 07.10.2026: общие базы уходят из продукта (#437), и выходом отказа
общая база больше не служит.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies),
[`cli.html#map`](../../../docs/site/cli.html#map).
