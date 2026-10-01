---
id: INV.CLI.CLONE-FROM-REFUSES-A-HELD-BASE-BEFORE-WRITING-THE-PROJECT
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/327
---

# `clone --from` отказывает на чужой базе до записи проекта

`clone --from` на файловой базе другой рабочей копии отказывает до того, как напишет
проектный файл и местный слой.

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies),
[`cli.html#map`](../../../docs/site/cli.html#map).
