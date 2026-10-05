---
id: INV.WIRE.A-MISSING-TOOL-IS-AN-ENVIRONMENT-FAILURE
check:
  - src/use_cases/result.rs::a_missing_utility_is_an_environment_failure
  - src/use_cases/result.rs::an_unsuitable_utility_version_is_an_environment_failure
  - src/mcp/error.rs::a_missing_utility_answers_a_runtime_failure_over_mcp
  - src/use_cases/load_artifact.rs::an_extension_load_without_ibcmd_answers_an_environment_failure
  - tests/cli_convert.rs::convert_without_the_edt_cli_answers_an_environment_failure
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
у MCP — `runtime_failure`.
