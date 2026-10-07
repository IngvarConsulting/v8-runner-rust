---
id: INV.CLI.A-FILE-COPY-IS-CREATED-BY-IBCMD
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/226
---

# Файловую копию базы создаёт `ibcmd`

`infobase create --from` создаёт файловую базу из образа источника через `ibcmd` — исполнителем
`infobase restore` с проверкой монопольного доступа. Пока этого исполнителя нет, база
создаётся Конфигуратором `/RestoreIB` (`INV.CLI.INFOBASE-CREATE-FROM-COPIES-A-BASE`).

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies), замер «Загрузка
информационной базы из DT» ([замеры](../../../references/1c/confirmed-runtime-measurements.md)).
