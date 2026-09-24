---
id: INV.WIRE.A-MISSING-TOOL-IS-AN-ENVIRONMENT-FAILURE
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/285
---

# Отказ из-за отсутствующей утилиты несёт род `environment`

Утилиты платформы нет в окружении — отказ несёт род `environment`: поставьте, и заработает.
Род `platform` оставлен за сбоем самой платформы.

Сегодня так отвечает выбор исполнителя по цепочке: когда не готов ни один кандидат, отказ —
`environment_unavailable` (`src/use_cases/provider_selection.rs`). Утилиту, которую ищут
напрямую, в обход цепочки, — например `1cedtcli` у `convert`, — отказ называет родом
`platform`: `AppError::PlatformLocator` отображается в `UseCaseErrorKind::Platform`
(`src/use_cases/result.rs`).
