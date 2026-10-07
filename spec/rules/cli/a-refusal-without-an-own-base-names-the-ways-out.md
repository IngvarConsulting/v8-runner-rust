---
id: INV.CLI.A-REFUSAL-WITHOUT-AN-OWN-BASE-NAMES-THE-WAYS-OUT
check:
  - tests/cli_infobases.rs::a_refusal_without_origin_names_the_ways_out
---

# Отказ без своей базы называет выходы

Отказ рабочей копии без объявленной базы (`infobases.origin` не объявлен) остаётся отказом и
называет выходы: своя чистая база (`init --infobase`, `infobase create`), копия базы
(`infobase create --from`) и база, развёрнутая из эталонного образа
(`infobase restore --create`). Общую базу выходом он не называет. Те же выходы называет
предупреждение о базе другой копии
(`INV.USE-CASES.A-WRITE-ON-A-BASE-OF-ANOTHER-COPY-RUNS-WITH-A-WARNING`).

Решение владельца от 07.10.2026: общих баз в продукте нет (#437), и выходом отказа общая база
не служит.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies),
[`cli.html#map`](../../../docs/site/cli.html#map).
