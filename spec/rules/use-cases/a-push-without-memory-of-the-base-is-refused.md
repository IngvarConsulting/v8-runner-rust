---
id: INV.USE-CASES.A-PUSH-WITHOUT-MEMORY-OF-THE-BASE-IS-REFUSED
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/215
---

# Отправка в базу без памяти о ней отказывает

`push` в базу, о которой у рабочей копии нет памяти, отказывает до запуска загрузки, в том
числе когда база пуста: пустую базу раннер не различает, потому что значение поколения у
пустой базы зависит от версии платформы ([замер](../../../references/1c/confirmed-runtime-measurements.md)). Отказ называет выходы: `pull`, если
права база, и `push --force`, если прав каталог. В базе под хранилищем конфигурации полная
загрузка невозможна, и выход `push --force` заменяет `pull --force`.

Выходы уточняют свои правила: на общей базе — `INV.USE-CASES.A-SHARED-BASE-REFUSAL-OFFERS-PULL-FIRST-AND-NAMES-PUSH`
(под хранилищем вместо `push --force` — `pull --force`), у копии и у нового владельца до первой отправки —
`INV.USE-CASES.A-COPIED-BASE-OFFERS-NO-PULL-BEFORE-ITS-FIRST-PUSH` и
`INV.USE-CASES.A-NEW-OWNER-IS-OFFERED-NO-PULL-UNTIL-ITS-FIRST-PUSH`. Памяти, записанной для
другой базы, отвечает `INV.USE-CASES.FOREIGN-MEMORY-IS-NOT-USED` со своими выходами. Базы,
которой нет, отказ не касается: выходы называет
`INV.CLI.A-REFUSAL-WITHOUT-AN-OWN-BASE-NAMES-THE-WAYS-OUT`.

Память о базе пишут её создание раннером (`INV.CLI.INFOBASE-CREATE-FOLLOWS-THE-TARGET-KIND`,
`INV.CLI.INFOBASE-CREATE-FROM-COPIES-A-BASE`) и полный `pull`
(`INV.USE-CASES.FULL-PULL-RECORDS-THE-PUBLISHED-TREE`), поэтому после них отказа нет.

Источник: [`cli.html#refusals`](../../../docs/site/cli.html#refusals).
