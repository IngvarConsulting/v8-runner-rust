---
id: INV.USE-CASES.A-PUSH-REFRESHES-THE-VERSION-FILE-ONLY-WHEN-IT-PAYS
check:
  - src/use_cases/build_project/coordinator.rs::a_full_or_a_large_matched_load_refreshes_the_version_file
  - tests/cli_push_generation.rs::a_full_push_refreshes_the_runner_copy
  - tests/cli_push_generation.rs::a_version_file_dumped_while_the_base_changed_is_not_recorded
  - tests/cli_push_generation.rs::a_full_ibcmd_push_refreshes_the_runner_copy_and_the_next_pull_goes_by_it
  - tests/cli_pull_memory.rs::a_push_leaves_the_runner_copy_and_passes_no_update_key
  - tests/cli_pull_memory.rs::an_agent_push_passes_no_update_key
  - tests/cli_pull_memory.rs::a_failed_push_does_not_change_the_runner_copy
  - tests/cli_build_agent.rs::a_managed_build_loads_and_updates_in_one_session_and_records_the_generation
---

# Загрузка получает свежий файл версий, только когда это окупается

`push` не передаёт загрузке ключ обновления описи (`-updateConfigDumpInfo` у Конфигуратора,
`--update-config-dump-info` у агента): платформа писала бы опись в каталог исходников. Свежую
опись загрузка получает отдельной выгрузкой одной описи во временный каталог под `workPath`
(`-configDumpInfoOnly`, `--config-dump-info-only`, `ibcmd config export info`) и делает её копией
раннера (`INV.USE-CASES.THE-VERSION-FILE-LIVES-ONLY-IN-THE-RUNNER-MEMORY`) только после удачной
полной загрузки или загрузки по изменившемуся не меньше чем из 1000 файлов, перед которой
поколение набора совпало с записанным; поколение читается до выгрузки описи и после, тем же
инструментом, и должно совпасть. Иначе — сдвиг поколения, нет ответа, сбой, отмена — копия
остаётся прежней, ответ это называет, а загрузка остаётся удачной. У `ibcmd` так только у
основной конфигурации: у расширения `export info` не замерен. Небольшая загрузка копию не
трогает: копия старше базы ровно на загруженное, и следующая выгрузка по изменившемуся выгрузит
эти объекты заново, ничего не пропустив.

## Замер

Запрос одной описи стоил отправке на КА 1,6–1,7 с, а выгрузка загруженного следующим `pull`
после 2, 30 и 300 общих модулей — 1,1–1,7 с; выгрузка дерева КА идёт около 5 мс на файл, и с
тысячи файлов обратная выгрузка однозначно дороже. После полной загрузки версии всех объектов
меняются, и следующая выгрузка по старой описи была бы полной
([замер](../../../references/1c/confirmed-runtime-measurements.md), раздел «Свежий файл версий
после крупной загрузки (#646)»).
