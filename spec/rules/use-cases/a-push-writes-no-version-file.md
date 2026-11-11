---
id: INV.USE-CASES.A-PUSH-WRITES-NO-VERSION-FILE
check:
  - tests/cli_pull_memory.rs::a_push_leaves_the_runner_copy_and_passes_no_update_key
  - tests/cli_pull_memory.rs::an_agent_push_passes_no_update_key
  - tests/cli_pull_memory.rs::a_failed_push_does_not_change_the_runner_copy
  - tests/cli_push_generation.rs::a_full_push_dumps_no_version_file_alone
  - tests/cli_push_generation.rs::a_full_ibcmd_push_writes_no_version_file
  - tests/cli_build_agent.rs::a_managed_build_loads_and_updates_in_one_session_and_records_the_generation
---

# Загрузка не пишет файл версий

`push` не передаёт загрузке `-updateConfigDumpInfo` у Конфигуратора и
`--update-config-dump-info` у агента, после загрузки не выгружает один файл версий
(`-configDumpInfoOnly`, `ibcmd config export info`) и копию раннера
(`INV.USE-CASES.THE-VERSION-FILE-LIVES-ONLY-IN-THE-RUNNER-MEMORY`) не трогает. Копия после
загрузки старше базы ровно на загруженное: следующая выгрузка по изменившемуся выгрузит эти
объекты заново, но ничего не пропустит. После полной загрузки версии всех объектов меняются, и
следующая выгрузка по изменившемуся на деле полная; без копии — полная по плану.

## Замер

Ключ `--update-config-dump-info` стоил загрузке на КА 1,8 с и следующей выгрузке времени не
экономил ([замер](../../../references/1c/confirmed-runtime-measurements.md), раздел «Файл
версий вне каталога исходников (#646)»).
