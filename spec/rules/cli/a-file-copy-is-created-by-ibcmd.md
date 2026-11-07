---
id: INV.CLI.A-FILE-COPY-IS-CREATED-BY-IBCMD
check:
  - tests/cli_infobase_copy.rs::a_file_copy_is_restored_by_ibcmd_without_changing_the_source
  - tests/cli_infobase_copy.rs::a_file_copy_selects_ibcmd_and_refuses_an_unavailable_executor
---

# Файловую копию базы создаёт `ibcmd`

На POSIX `infobase create --from` по умолчанию создаёт файловую базу из образа источника
через `ibcmd` — исполнителем `infobase restore` с проверкой отсутствия сеансов. Явный
`providers.infobase.create: designer` выбирает Конфигуратор `/RestoreIB`. На Windows
Конфигуратор остаётся умолчанием, пока признак сеансов для `ibcmd` там не измерен
(`INV.CLI.INFOBASE-CREATE-FROM-COPIES-A-BASE`).

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies), замер «Загрузка
информационной базы из DT» ([замеры](../../../references/1c/confirmed-runtime-measurements.md)).
