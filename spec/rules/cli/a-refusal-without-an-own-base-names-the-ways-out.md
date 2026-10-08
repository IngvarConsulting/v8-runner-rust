---
id: INV.CLI.A-REFUSAL-WITHOUT-AN-OWN-BASE-NAMES-THE-WAYS-OUT
check:
  - tests/cli_infobases.rs::a_refusal_without_origin_names_the_ways_out
  - src/use_cases/infobase_owner.rs::the_refusal_and_the_warning_name_the_same_ways_out
  - tests/architecture_guardrails.rs::the_ways_to_an_own_infobase_have_one_builder
---

# Отказ без своей базы называет выходы

Отказ рабочей копии без объявленной базы (`infobases.origin` не объявлен) остаётся отказом и
называет выходы: своя чистая база (`init --infobase`, `infobase create`), копия базы
(`infobase create --from`) и база, развёрнутая из эталонного образа
(`infobase restore --create`). Те же выходы называет
предупреждение о базе другой копии
(`INV.USE-CASES.A-WRITE-ON-A-BASE-OF-ANOTHER-COPY-RUNS-WITH-A-WARNING`).

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies),
[`cli.html#map`](../../../docs/site/cli.html#map).
