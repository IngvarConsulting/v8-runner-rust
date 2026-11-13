---
id: INV.WIRE.A-MISSING-TOOL-IS-AN-ENVIRONMENT-FAILURE
check:
  - src/use_cases/result.rs::a_missing_utility_is_an_environment_failure
  - src/use_cases/result.rs::an_unsuitable_utility_version_is_an_environment_failure
  - src/mcp/error.rs::a_missing_utility_answers_a_runtime_failure_over_mcp
  - src/mcp/error.rs::a_missing_release_answers_a_runtime_failure_with_its_inventory_over_mcp
  - src/command_envelope.rs::every_error_kind_and_code_is_named_by_the_schema_and_by_a_table
  - src/use_cases/load_artifact.rs::an_extension_load_without_ibcmd_answers_an_environment_failure
  - tests/cli_convert.rs::convert_without_the_edt_cli_answers_an_environment_failure
  - tests/cli_convert.rs::convert_a_package_direction_without_ibcmd_answers_an_environment_failure
  - src/use_cases/infobase_export.rs::a_dump_without_its_utility_records_an_environment_step_code
  - src/use_cases/infobase_export.rs::the_step_code_follows_the_envelope_kind_for_every_error
  - tests/cli_infobase.rs::infobase_dump_without_its_utility_records_an_environment_step_code
  - src/cli/output.rs::a_missing_utility_names_the_platform_layer_and_a_missing_release_names_none
  - tests/cli_convert.rs::convert_without_the_edt_cli_answers_an_environment_failure
  - tests/cli_dump_agent.rs::a_managed_agent_without_the_local_platform_is_refused_before_any_connection
---

# Отказ из-за отсутствующей утилиты несёт род `environment`

Подходящей утилиты платформы нет в окружении — её не нашли, или нашли не той версии, или её
версию не прочитать, — отказ несёт род `environment`: поставьте подходящую, и заработает.
Род `platform` оставлен за сбоем самой платформы.

Так отвечает и выбор исполнителя по цепочке, когда не готов ни один кандидат
(`src/use_cases/provider_selection.rs`), и утилита, которую ищут напрямую, в обход цепочки, —
например `1cedtcli` у `convert` или `ibcmd`, которым `load` спрашивает перечень расширений:
`AppError::PlatformLocator` отображается в `UseCaseErrorKind::Environment`
(`src/use_cases/result.rs`). В конверте CLI это код `environment_unavailable` и выход 2,
у MCP — `runtime_failure`. Слой у такого отказа — `platform`: поле `layer` в конверте CLI и в
отказе MCP (`INV.WIRE.AN-ENVIRONMENT-REFUSAL-NAMES-ITS-LAYER`). Выбор исполнителя называет
этот слой, когда хоть одному кандидату не нашлось подходящей утилиты; если все пропущены по
другим причинам, слой не определён и поля нет.

Род один, а код уточняет причина (`EnvironmentReason`): утилиты нет или она не та —
`environment_unavailable`; при `tools.platform.version: sources` нет выпуска формата
исходников — `platform_release_missing`, тоже выход 2, с полем `platform_release`
(`INV.PLATFORM.A-MISSING-RELEASE-OF-THE-SOURCES-FORMAT-IS-REFUSED`). MCP различает их только
полем: код у каждой из них `runtime_failure`, род `runtime`, а `platform_release` едет в отказе MCP
так же, как в конверте CLI. Поля `layer` у `platform_release_missing` нет: у него свой код.

Код шага в `data.execution.errors[]` у `download`, `infobase dump` и `infobase restore`
выводится из того же рода, поэтому недостающая утилита пишет там `environment_unavailable`, а
не `platform_failure`; отсутствующий выпуск формата исходников пишет там тот же
`environment_unavailable`: причина рода коду шага не видна. Своего отображения ошибок в код у шага нет: род отказа
(`UseCaseErrorKind::of`) — единственный владелец, а код шага — `execution_step_code` этого
рода.
