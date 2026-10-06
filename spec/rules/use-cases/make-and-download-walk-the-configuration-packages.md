---
id: INV.USE-CASES.MAKE-AND-DOWNLOAD-WALK-THE-CONFIGURATION-PACKAGES
check: []
gap: https://github.com/IngvarConsulting/v8-runner-rust/issues/364
---

# `make` и `download` без набора обходят пакеты порядком инвентаря

`make` и `download` без набора идут по пакетам конфигурации проекта через
`SourceSetInventory::configuration_packages`, тем же порядком, что `pull --all`.
