---
id: INV.USE-CASES.THE-DUMP-MODE-IS-FORECAST-IN-THE-SAME-COMMAND
check:
  - src/platform/dump_forecast.rs::the_changes_list_is_read_by_its_measured_keys
  - src/platform/dump_forecast.rs::the_export_status_is_read_by_its_measured_keys
  - src/use_cases/dump_config.rs::a_designer_full_forecast_is_reported_as_full_by_the_platform
  - src/use_cases/dump_config.rs::an_unrecognized_forecast_reports_the_mode_as_unknown
  - src/use_cases/dump_config.rs::a_changes_forecast_keeps_the_incremental_mode
  - src/use_cases/dump_config.rs::a_failed_forecast_reports_the_mode_as_unknown
  - src/use_cases/dump_config.rs::a_dump_without_a_version_file_asks_for_no_forecast
  - src/use_cases/dump_config.rs::an_ibcmd_full_forecast_dumps_through_the_stage_without_sync
  - src/use_cases/dump_config.rs::an_ibcmd_partial_dump_follows_a_full_forecast_too
  - tests/cli_push_generation.rs::a_designer_pull_with_an_unchanged_generation_dumps_nothing
  - tests/cli_dump_agent.rs::a_generation_recorded_by_another_tool_does_not_skip_a_dump
  - tests/cli_dump_agent.rs::an_unchanged_generation_skips_an_incremental_dump
---

# Режим выгрузки предсказывается в той же команде

Перед выгрузкой по изменившемуся с годным файлом версий раннер в той же команде и под тем же
замком спрашивает у платформы прогноз: у Конфигуратора — `-getChanges`, у агента —
`--get-changes` в той же сессии, у `ibcmd` — `config export status`; у расширения так же, с его
именем. Выгрузка, которую раннер пропускает по неизменному поколению, прогноза не спрашивает.
Прогноз «полная» (`FullDump`, `modified: all`) ответ называет случившимся режимом `FULL` с
причиной `platform_forecast`; Конфигуратор и агент при этом выгружают `-update` как есть, а
`ibcmd`, чей `--sync` полную выгрузку не делает и отказывает, выгружает через промежуточный
каталог (`INV.USE-CASES.AN-IBCMD-FULL-DUMP-LANDS-OVER-THE-DIRECTORY-THROUGH-A-STAGE`) — так же и
выборка `ibcmd`, которая идёт `--sync`. Прогноз, которого нет в замеренном словаре, или
отказ прогноза дают режим `UNKNOWN` с причиной `unknown`, а не «инкрементальный»; выгрузка
при этом идёт как просили. Ответ обещает режим по прогнозу и не больше: базу между прогнозом и
выгрузкой может изменить тот, кого замок раннера не держит
([замер](../../../references/1c/confirmed-runtime-measurements.md)). Без файла версий или с
чужим прогноз не нужен: выгрузка полная по
`INV.USE-CASES.A-MISSING-VERSION-FILE-IS-KNOWN-BEFORE-THE-PLATFORM-STARTS` и
`INV.USE-CASES.A-FOREIGN-FORMAT-VERSION-TURNS-THE-DUMP-FULL`. Превью платформу не запускает и
называет режим плана. Пару «запрошенный и случившийся режим» несёт форма `CTR.WIRE.PULL-DATA`.
