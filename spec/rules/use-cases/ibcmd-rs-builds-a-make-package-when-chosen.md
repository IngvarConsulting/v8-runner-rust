---
id: INV.USE-CASES.IBCMD-RS-BUILDS-A-MAKE-PACKAGE-WHEN-CHOSEN
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/413
---

# `ibcmd-rs` собирает пакет `make`, когда его назначили

`providers.make: ibcmd-rs` собирает `.cf` и `.cfe` из XML без базы и без платформы. В
цепочку умолчаний `make` он не входит: его байтовая точность ниже родной выгрузки
([замер](../../../references/1c/confirmed-runtime-measurements.md)). Пока вызов, входы и
выходы, поддержка `.cfe` и работа в Windows не замерены, адаптера нет, и ключ отказывает
при валидации конфигурации.
