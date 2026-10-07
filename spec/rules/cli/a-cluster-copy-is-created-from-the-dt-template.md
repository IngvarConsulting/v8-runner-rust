---
id: INV.CLI.A-CLUSTER-COPY-IS-CREATED-FROM-THE-DT-TEMPLATE
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/434
---

# Копию базы в кластере Конфигуратор создаёт по шаблону .dt

`infobase create --from` создаёт базу в кластере одной командой Конфигуратора
`CREATEINFOBASE` с шаблоном — образом источника (`/UseTemplate <файл>.dt`). Пока вызов не
замерен, база создаётся замеренным путём: `CREATEINFOBASE`, затем `/RestoreIB`
(`INV.CLI.INFOBASE-CREATE-FROM-COPIES-A-BASE`).

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies), документация
[`CREATEINFOBASE`](../../../references/1c/designer-startup/mode-selection/ZIF1.md).
