---
id: INV.WIRE.A-MISSING-TOOL-IS-AN-ENVIRONMENT-FAILURE
check:
  - src/use_cases/result.rs::a_missing_utility_is_an_environment_failure
  - tests/cli_convert.rs::convert_without_the_edt_cli_answers_an_environment_failure
---

# Отказ из-за отсутствующей утилиты несёт род `environment`

Утилиты платформы нет в окружении — отказ несёт род `environment`: поставьте, и заработает.
Род `platform` оставлен за сбоем самой платформы.

Так отвечает и выбор исполнителя по цепочке, когда не готов ни один кандидат
(`src/use_cases/provider_selection.rs`), и утилита, которую ищут напрямую, в обход цепочки, —
например `1cedtcli` у `convert`: `AppError::PlatformLocator` отображается в
`UseCaseErrorKind::Environment` (`src/use_cases/result.rs`). В конверте CLI это код
`environment_unavailable` и выход 2, у MCP — `runtime_failure`.
