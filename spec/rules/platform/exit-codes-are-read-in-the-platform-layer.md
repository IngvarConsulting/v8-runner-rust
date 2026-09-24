---
id: INV.PLATFORM.EXIT-CODES-ARE-READ-IN-THE-PLATFORM-LAYER
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/285
---

# Код выхода утилиты читает слой платформы

Что значит код выхода утилиты платформы, решает слой `platform`: это знание об инструменте —
у `/CompareCfg` ноль значит «сравнение состоялось», — и живёт оно рядом с адаптером.
Сценарий получает исход, а не код.

Сегодня сценарии сравнивают код выхода с нулём сами: `configure_extensions.rs`,
`extension_inventory.rs`, `convert_sources.rs` и `artifacts.rs` в `src/use_cases/`.
