---
id: INV.CLI.A-FILE-COPY-IS-CREATED-BY-IBCMD
check:
  - tests/cli_infobase_copy.rs::a_file_copy_is_restored_by_ibcmd_without_changing_the_source
  - tests/cli_infobase_copy.rs::a_file_copy_selects_ibcmd_and_refuses_an_unavailable_executor
  - tests/cli_infobase_copy.rs::a_file_copy_without_ibcmd_falls_back_to_the_designer
  - src/use_cases/init_project.rs::a_copy_restores_with_the_chain_its_adapters_can_run
---

# Файловую копию базы создаёт `ibcmd`

На POSIX `infobase create --from` по умолчанию создаёт файловую базу из образа источника
через `ibcmd` — исполнителем `infobase restore` с проверкой отсутствия сеансов. Исполнитель
выбирается по готовности из строки `infobase create`: без найденного `ibcmd` и превью, и
прогон берут Конфигуратор `/RestoreIB`, а квитанция называет пропущенный `ibcmd` с причиной.
Явный `providers.infobase.create: designer` выбирает Конфигуратор. На Windows `ibcmd` из
цепочки копии исключён с причиной от его адаптера, пока признак сеансов там не измерен
(`INV.CLI.INFOBASE-CREATE-FROM-COPIES-A-BASE`).

Источник: [`sources.html#copies`](../../../docs/site/sources.html#copies), замер «Загрузка
информационной базы из DT» ([замеры](../../../references/1c/confirmed-runtime-measurements.md)).
